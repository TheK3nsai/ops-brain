#!/usr/bin/env python3
"""Local fixture tests; no deployment, server, credentials or external services."""

import importlib.util
import io
import json
from pathlib import Path
import stat
import tempfile
import unittest


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


setup = load("setup-self-host")
smoke = load("mcp-smoke")


class SetupTests(unittest.TestCase):
    def test_private_files_and_existing_credentials_preserved(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "deployment"
            env, tokens = setup.initialize(directory, ["Claude-Laptop", "Codex-Buildbox"], "v6.0.0")
            original = env.read_bytes()
            self.assertEqual(stat.S_IMODE(env.stat().st_mode), 0o600)
            self.assertEqual(stat.S_IMODE(tokens.stat().st_mode), 0o700)
            first = (tokens / "Claude-Laptop").read_text()
            second = (tokens / "Codex-Buildbox").read_text()
            self.assertNotEqual(first, second)
            self.assertEqual(len(first), 64)
            self.assertEqual(stat.S_IMODE((tokens / "Claude-Laptop").stat().st_mode), 0o600)
            bindings = next(line for line in env.read_text().splitlines() if line.startswith("OPS_BRAIN_AGENT_TOKENS="))
            self.assertEqual(json.loads(bindings.split("=", 1)[1].strip("'"))[0]["token"], first)
            with self.assertRaises(ValueError):
                setup.initialize(directory, ["Other-Agent"], "v6.0.0")
            self.assertEqual(env.read_bytes(), original)

    def test_rejects_duplicate_slugs_and_symlinks(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "deployment"
            with self.assertRaises(ValueError):
                setup.initialize(directory, ["Bot", "bot"], "v6.0.0")
            directory.symlink_to(Path(temporary), target_is_directory=True)
            with self.assertRaises(ValueError):
                setup.initialize(directory, ["Bot"], "v6.0.0")
            self.assertFalse((Path(temporary) / ".env").exists())


class Response(io.BytesIO):
    def __init__(self, data, headers):
        super().__init__(data)
        self.headers = headers


class Opener:
    def __init__(self, response):
        self.response = response
        self.request = None

    def open(self, request, timeout):
        self.request = request
        return self.response


class SmokeTests(unittest.TestCase):
    def test_remote_plaintext_and_credential_urls_rejected(self):
        for url in ["http://example.com/mcp", "https://user:token@example.com/mcp", "https://example.com/mcp?token=x"]:
            with self.assertRaises(ValueError):
                smoke.check_url(url)
        smoke.check_url("https://example.com/mcp")
        smoke.check_url("http://127.0.0.1:3000/mcp")

    def test_sse_notifications_skipped_session_retained(self):
        client = smoke.MCP("https://example.com/mcp", "fixture-token")
        client.opener = Opener(Response(
            b'data: {"jsonrpc":"2.0","method":"notifications/message"}\n\n'
            b'data: {"jsonrpc":"2.0","id":1,"result":{"tools":[]}}\n\n',
            {"Content-Type": "text/event-stream", "Mcp-Session-Id": "fixture-session"}))
        self.assertEqual(client.send("tools/list"), {"tools": []})
        self.assertEqual(client.session, "fixture-session")
        self.assertEqual(client.opener.request.get_header("Authorization"), "Bearer fixture-token")

    def test_errors_do_not_echo_upstream_data(self):
        client = smoke.MCP("https://example.com/mcp", "fixture-token")
        client.opener = Opener(Response(
            b'{"jsonrpc":"2.0","id":1,"error":{"message":"private-upstream-data"}}',
            {"Content-Type": "application/json"}))
        with self.assertRaises(RuntimeError) as caught:
            client.send("tools/list")
        self.assertNotIn("private-upstream-data", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
