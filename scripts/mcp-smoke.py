#!/usr/bin/env python3
"""Check a legacy Streamable HTTP MCP connection; optional isolated handoff exercise."""

import argparse
import json
import os
from pathlib import Path
import sys
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener
import uuid


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class MCP:
    def __init__(self, url, token):
        self.url, self.token, self.session = url, token, None
        self.sequence = 0
        self.opener = build_opener(NoRedirect())

    def send(self, method, params=None, notification=False):
        payload = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            payload["params"] = params
        if not notification:
            self.sequence += 1
            payload["id"] = self.sequence
        headers = {"Authorization": f"Bearer {self.token}", "Content-Type": "application/json",
                   "Accept": "application/json, text/event-stream", "User-Agent": "ops-brain-mcp-smoke"}
        if self.session:
            headers["Mcp-Session-Id"] = self.session
            headers["MCP-Protocol-Version"] = "2025-11-25"
        request = Request(self.url, json.dumps(payload).encode(), headers, method="POST")
        try:
            with self.opener.open(request, timeout=30) as response:
                self.session = response.headers.get("Mcp-Session-Id", self.session)
                if notification:
                    return None
                if "text/event-stream" in response.headers.get("Content-Type", ""):
                    # Consume one JSON-RPC response without waiting for the SSE connection to close.
                    chunks, read_bytes = [], 0
                    while True:
                        line = response.readline(1_048_577)
                        read_bytes += len(line)
                        if read_bytes > 1_048_576:
                            raise RuntimeError("MCP response exceeds 1 MiB")
                        if not line:
                            raise RuntimeError("SSE ended without a matching response")
                        if line.strip() == b"":
                            if chunks:
                                data = b"\n".join(chunks)
                                if not data.strip():
                                    chunks = []
                                    continue
                                value = json.loads(data)
                                if value.get("id") == payload["id"]:
                                    break
                                chunks = []
                        elif line.startswith(b"data:"):
                            chunks.append(line[5:].strip())
                else:
                    data = response.read(1_048_577)
                    if len(data) > 1_048_576:
                        raise RuntimeError("MCP response exceeds 1 MiB")
                    value = json.loads(data)
        except HTTPError as error:
            # Do not print upstream bodies, headers, or URLs that might contain secrets.
            raise RuntimeError(f"HTTP {error.code} during {method}") from None
        except (URLError, TimeoutError):
            raise RuntimeError(f"Connection failed during {method}") from None
        if value.get("id") != payload["id"] or "error" in value:
            raise RuntimeError(f"JSON-RPC failed during {method}")
        return value["result"]

    def call(self, name, arguments, expect_error=False):
        result = self.send("tools/call", {"name": name, "arguments": arguments})
        if bool(result.get("isError")) != expect_error:
            raise RuntimeError(f"Unexpected tool outcome for {name}")
        if expect_error:
            return result
        if "structuredContent" in result:
            return result["structuredContent"]
        for block in result.get("content", []):
            if block.get("type") == "text":
                return json.loads(block["text"])
        raise RuntimeError(f"Missing structured result for {name}")


def check_url(url):
    parsed = urlsplit(url)
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError("URL must not contain credentials, query parameters, or a fragment")
    if not parsed.hostname or parsed.scheme not in ("http", "https"):
        raise ValueError("supply an HTTP(S) MCP endpoint")
    if parsed.scheme == "http" and parsed.hostname not in ("localhost", "127.0.0.1", "::1"):
        raise ValueError("remote MCP endpoints require HTTPS")


def exercise(client, agent):
    run = uuid.uuid4().hex
    arguments = {"from_agent": agent, "to_agent": agent, "category": "notify",
                 "title": "MCP compatibility smoke test", "body": "Disposable connection test."}
    handoff = client.call("create_handoff", arguments)
    identifier = handoff["id"]
    wrong = dict(arguments, from_agent=f"Probe-{run}")
    client.call("create_handoff", wrong, expect_error=True)
    client.call("get_handoff", {"handoff_id": identifier})
    client.call("accept_handoff", {"handoff_id": identifier, "agent_name": agent})
    reply = client.call("create_handoff", dict(arguments, in_reply_to=identifier,
                         title="MCP smoke test reply"))
    client.call("complete_handoff", {"handoff_id": reply["id"]})
    client.call("complete_handoff", {"handoff_id": identifier})
    return identifier


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True)
    parser.add_argument("--token-file", type=Path, help="private file containing this agent's bearer")
    parser.add_argument("--agent", required=True)
    parser.add_argument("--exercise", action="store_true", help="create and complete two test notifications")
    args = parser.parse_args()
    try:
        check_url(args.url)
        token = args.token_file.read_text().strip() if args.token_file else os.environ.get("OPS_BRAIN_AGENT_TOKEN", "")
        if not token or any(c.isspace() for c in token):
            raise ValueError("supply a bearer using --token-file or OPS_BRAIN_AGENT_TOKEN")
        client = MCP(args.url, token)
        initialized = client.send("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                        "clientInfo": {"name": "ops-brain-smoke", "version": "1"}})
        if initialized.get("protocolVersion") != "2025-11-25":
            raise RuntimeError("Server did not negotiate the requested protocol version")
        client.send("notifications/initialized", notification=True)
        catalog = client.send("tools/list")
        names = {tool["name"] for tool in catalog["tools"]}
        if not {"check_in", "create_handoff", "get_handoff", "accept_handoff", "complete_handoff"} <= names:
            raise RuntimeError("Required coordination tools are missing")
        client.call("check_in", {"agent_name": args.agent})
        if args.exercise:
            identifier = exercise(client, args.agent)
            print(f"PASS: handoff/thread/identity checks; completed test handoff {identifier}")
        else:
            print(f"PASS: initialize, {len(names)} tools, and check_in (read only)")
    except (OSError, ValueError, KeyError, RuntimeError) as error:
        # OSError paths may be useful; token data and response bodies are never printed.
        parser.exit(1, f"Smoke check failed: {error}\n")


if __name__ == "__main__":
    sys.exit(main())
