#!/usr/bin/env python3
"""Isolated HTTPS OAuth/PKCE + MCP handoff fixture. Never uses deployment credentials."""

import argparse
import base64
import contextlib
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
import os
from pathlib import Path
import queue
import re
import secrets
import select
import shutil
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import traceback
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs, urlencode, urlsplit
from urllib.request import HTTPSHandler, Request, build_opener
import uuid


REPO = Path(__file__).resolve().parents[1]
STAGE = "initialization"
spec = importlib.util.spec_from_file_location("smoke", REPO / "scripts/mcp-smoke.py")
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


def run(command, **kwargs):
    result = subprocess.run(command, capture_output=True, **kwargs)
    if result.returncode:
        # Child output may contain URLs, codes, credentials or private key PEM.
        raise RuntimeError(f"fixture command failed: {Path(command[0]).name}")
    return result.stdout


def b64(value):
    return base64.urlsafe_b64encode(value).rstrip(b"=").decode()


def free_port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


class FixtureServer(ThreadingHTTPServer):
    def handle_error(self, request, client_address):
        # The caller will fail its expected response. Do not dump request
        # processing context (potentially carrying fixture credentials).
        pass


class Provider:
    """Test-only issuer: auto-consents a synthetic user for two fixture clients."""

    def __init__(self, root):
        self.root, self.codes, self.tokens = root, {}, {}
        self.authenticated_initializations = {}
        self.lock = threading.Lock()
        self.key, self.certificate = root / "jwt-key.pem", root / "ca.pem"
        ca_key, tls_key, tls_cert = root / "ca-key.pem", root / "tls-key.pem", root / "tls.pem"
        run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
             "-keyout", str(ca_key), "-out", str(self.certificate), "-days", "1",
             "-subj", "/CN=OAuth fixture CA", "-addext", "basicConstraints=critical,CA:TRUE",
             "-addext", "keyUsage=critical,keyCertSign,cRLSign"])
        run(["openssl", "req", "-new", "-newkey", "rsa:2048", "-nodes", "-keyout", str(tls_key),
             "-out", str(root / "tls.csr"), "-subj", "/CN=localhost"])
        extensions = root / "tls-extensions.conf"
        extensions.write_text("basicConstraints=critical,CA:FALSE\n"
                              "subjectAltName=DNS:localhost,IP:127.0.0.1\n"
                              "keyUsage=critical,digitalSignature,keyEncipherment\n"
                              "extendedKeyUsage=serverAuth\n")
        run(["openssl", "x509", "-req", "-in", str(root / "tls.csr"), "-CA", str(self.certificate),
             "-CAkey", str(ca_key), "-CAcreateserial", "-out", str(tls_cert), "-days", "1",
             "-extfile", str(extensions)])
        run(["openssl", "genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", str(self.key)])
        for key in (self.key, ca_key, tls_key):
            key.chmod(0o600)
        modulus = run(["openssl", "rsa", "-in", str(self.key), "-modulus", "-noout"])
        modulus = bytes.fromhex(modulus.decode().strip().split("=", 1)[1])
        self.jwks = {"keys": [{"kty": "RSA", "kid": "fixture", "alg": "RS256", "use": "sig",
                               "n": b64(modulus), "e": "AQAB"}]}
        self.context = ssl.create_default_context(cafile=str(self.certificate))
        self.opener = build_opener(smoke.NoRedirect(), HTTPSHandler(context=self.context))
        self.proxy = f"http://127.0.0.1:{free_port()}"
        self.server = FixtureServer(("127.0.0.1", 0), self.handler())
        self.issuer = f"https://localhost:{self.server.server_port}"
        self.resource = self.issuer + "/mcp"
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(str(tls_cert), str(tls_key))
        self.server.socket = context.wrap_socket(self.server.socket, server_side=True)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def token(self, client, **overrides):
        now = int(time.time())
        claims = {"iss": self.issuer, "sub": "fixture-user", "client_id": client,
                  "aud": self.resource, "iat": now, "exp": now + 300,
                  "jti": uuid.uuid4().hex, "scope": "ops-brain"}
        claims.update(overrides)
        message = (b64(json.dumps({"alg": "RS256", "kid": "fixture", "typ": "at+jwt"}).encode())
                   + "." + b64(json.dumps(claims).encode()))
        signature = run(["openssl", "dgst", "-sha256", "-sign", str(self.key)], input=message.encode())
        return message + "." + b64(signature)

    def handler(self):
        provider = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass  # Never log auth headers, authorization codes or callback URLs.

            def reply(self, status, value, headers=None):
                data = json.dumps(value).encode() if value is not None else b""
                self.send_response(status)
                for name, value in (headers or {}).items():
                    self.send_header(name, value)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                path = urlsplit(self.path).path
                if path == "/.well-known/oauth-authorization-server":
                    return self.reply(200, {"issuer": provider.issuer,
                        "authorization_endpoint": provider.issuer + "/authorize",
                        "token_endpoint": provider.issuer + "/token",
                        "jwks_uri": provider.issuer + "/jwks",
                        "response_types_supported": ["code"],
                        "grant_types_supported": ["authorization_code"],
                        "token_endpoint_auth_methods_supported": ["none"],
                        "code_challenge_methods_supported": ["S256"],
                        "authorization_response_iss_parameter_supported": True,
                        "scopes_supported": ["ops-brain"]})
                if path == "/jwks":
                    return self.reply(200, provider.jwks)
                if path == "/authorize":
                    values = {key: value[0] for key, value in parse_qs(urlsplit(self.path).query).items()}
                    callback = urlsplit(values.get("redirect_uri", ""))
                    if (values.get("client_id") not in ("codex-fixture", "claude-fixture")
                            or values.get("resource") != provider.resource
                            or values.get("response_type") != "code"
                            or values.get("code_challenge_method") != "S256"
                            or not values.get("code_challenge") or not values.get("state")
                            or callback.scheme != "http" or callback.hostname not in ("127.0.0.1", "localhost")
                            or callback.username or callback.password or callback.fragment):
                        return self.reply(400, {"error": "invalid_request"})
                    code = secrets.token_urlsafe(32)
                    with provider.lock:
                        provider.codes[code] = (values, time.monotonic() + 30)
                    separator = "&" if callback.query else "?"
                    location = values["redirect_uri"] + separator + urlencode({
                        "code": code, "state": values["state"], "iss": provider.issuer})
                    return self.reply(302, None, {"Location": location})
                return self.proxy_request()

            def do_POST(self):
                if urlsplit(self.path).path != "/token":
                    return self.proxy_request()
                values = {key: value[0] for key, value in parse_qs(
                    self.rfile.read(int(self.headers.get("Content-Length", "0"))).decode()).items()}
                with provider.lock:
                    code = provider.codes.pop(values.get("code"), None)
                if not code:
                    return self.reply(400, {"error": "invalid_grant"})
                authorized, expiry = code
                challenge = b64(hashlib.sha256(values.get("code_verifier", "").encode()).digest())
                if (expiry <= time.monotonic() or values.get("grant_type") != "authorization_code"
                        or any(values.get(key) != authorized.get(key) for key in
                               ("client_id", "redirect_uri", "resource"))
                        or challenge != authorized["code_challenge"]):
                    return self.reply(400, {"error": "invalid_grant"})
                token = provider.token(values["client_id"])
                with provider.lock:
                    provider.tokens[values["client_id"]] = token
                return self.reply(200, {"access_token": token, "token_type": "Bearer",
                                        "expires_in": 300, "scope": "ops-brain"})

            def proxy_request(self):
                data = self.rfile.read(int(self.headers.get("Content-Length", "0")))
                headers = {name: value for name, value in self.headers.items()
                           if name.lower() not in ("host", "content-length", "connection")}
                headers["Host"] = urlsplit(provider.issuer).netloc
                request = Request(provider.proxy + self.path, data if self.command == "POST" else None,
                                  headers, method=self.command)
                try:
                    response = build_opener(smoke.NoRedirect()).open(request, timeout=15)
                except HTTPError as error:
                    response = error
                except URLError:
                    return self.reply(503, {"error": "fixture_backend_unavailable"})
                with response:
                    if response.status == 200 and data and json.loads(data).get("method") == "initialize":
                        with provider.lock:
                            for client, token in provider.tokens.items():
                                if self.headers.get("Authorization") == "Bearer " + token:
                                    provider.authenticated_initializations[client] = (
                                        provider.authenticated_initializations.get(client, 0) + 1)
                    if "text/event-stream" in response.headers.get("Content-Type", ""):
                        # MCP legacy transport keeps SSE open; forward just this
                        # request's JSON-RPC response, never wait for stream EOF.
                        expected_id = json.loads(data)["id"]
                        chunks, total = [], 0
                        while True:
                            line = response.readline(1_048_577)
                            total += len(line)
                            if not line or total > 1_048_576:
                                raise RuntimeError("fixture proxy received invalid SSE")
                            if not line.strip() and chunks:
                                body = b"\n".join(chunks)
                                if not body.strip():
                                    chunks = []
                                    continue
                                if json.loads(body).get("id") == expected_id:
                                    break
                                chunks = []
                            elif line.startswith(b"data:"):
                                chunks.append(line[5:].strip())
                    else:
                        body = response.read(1_048_577)
                    self.send_response(response.status)
                    for name in ("Mcp-Session-Id", "WWW-Authenticate"):
                        if response.headers.get(name):
                            self.send_header(name, response.headers[name])
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)

        return Handler

    def authorize(self, client, invalid_verifier=False):
        verifier, state = secrets.token_urlsafe(48), secrets.token_urlsafe(32)
        values = {"client_id": client, "redirect_uri": "http://127.0.0.1:8765/callback",
                  "resource": self.resource, "scope": "ops-brain", "response_type": "code",
                  "code_challenge_method": "S256", "state": state,
                  "code_challenge": b64(hashlib.sha256(verifier.encode()).digest())}
        try:
            self.opener.open(self.issuer + "/authorize?" + urlencode(values), timeout=10)
            raise RuntimeError("expected authorization redirect")
        except HTTPError as response:
            assert response.code == 302
            callback = parse_qs(urlsplit(response.headers["Location"]).query)
            assert callback["state"] == [state] and callback["iss"] == [self.issuer]
        form = {key: values[key] for key in ("client_id", "redirect_uri", "resource")}
        form.update(grant_type="authorization_code", code=callback["code"][0], code_verifier=verifier)
        if invalid_verifier:
            form["code_verifier"] = secrets.token_urlsafe(48)
        request = Request(self.issuer + "/token", urlencode(form).encode())
        if invalid_verifier:
            try:
                self.opener.open(request, timeout=10)
                raise RuntimeError("invalid PKCE verifier was accepted")
            except HTTPError as response:
                assert response.code == 400
                return None
        with self.opener.open(request, timeout=10) as response:
            token = json.load(response)["access_token"]
        try:
            self.opener.open(request, timeout=10)
            raise RuntimeError("authorization code replay was accepted")
        except HTTPError as response:
            assert response.code == 400
        return token

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


def mcp(provider, token):
    client = smoke.MCP(provider.resource, token)
    client.opener = provider.opener
    client.send("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                "clientInfo": {"name": "oauth-fixture", "version": "1"}})
    client.send("notifications/initialized", notification=True)
    assert len(client.send("tools/list")["tools"]) == 13
    return client


def exercise(provider, codex_token, recipient=None):
    provider.authorize("claude-fixture", invalid_verifier=True)
    sender = mcp(provider, provider.authorize("claude-fixture"))
    recipient = recipient or mcp(provider, codex_token)
    arguments = {"from_agent": "Fixture-Claude", "to_agent": "Fixture-Codex",
                 "title": "OAuth fixture " + uuid.uuid4().hex,
                 "body": "Synthetic local integration test."}
    sender.call("create_handoff", dict(arguments, from_agent="Forged-Agent"), expect_error=True)
    handoff = sender.call("create_handoff", arguments)
    identifier = handoff["id"]
    claimed = recipient.call("accept_handoff", {"handoff_id": identifier, "agent_name": "Fixture-Codex"})
    assert claimed["accepted_by"] == "Fixture-Codex"
    recipient.call("check_in", {"agent_name": "Fixture-Claude"}, expect_error=True)
    recipient.call("add_knowledge", {"author": "Fixture-Codex", "title": "refuse client data",
                    "content": "fixture", "category": "gotcha", "client_slug": "private-client"}, expect_error=True)
    reply = recipient.call("create_handoff", dict(arguments, from_agent="Fixture-Codex",
                            to_agent="Fixture-Claude", category="notify", in_reply_to=identifier))
    recipient.call("complete_handoff", {"handoff_id": identifier})
    sender.call("complete_handoff", {"handoff_id": reply["id"]})
    replies = sender.call("list_replies_to_me", {"agent_name": "Fixture-Claude"})
    assert any(row["id"] == reply["id"] for row in replies["replies"])
    assert sender.call("get_handoff", {"handoff_id": identifier})["status"] == "completed"
    for token, path, status in (
            (codex_token, "/api/briefing", 401),
            (provider.token("codex-fixture", exp=int(time.time()) - 1), "/mcp", 401),
            (provider.token("codex-fixture", aud="other-api"), "/mcp", 401)):
        request = Request(provider.issuer + path, b"{}", {"Authorization": "Bearer " + token})
        try:
            provider.opener.open(request, timeout=10)
            raise RuntimeError("invalid OAuth request was accepted")
        except HTTPError as response:
            assert response.code == status


def fixture_package(root, target, provider):
    spec = importlib.util.spec_from_file_location("packaging", REPO / "scripts/package-plugin.py")
    packaging = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(packaging)
    output = root / (target + "-packages")
    packaging.build(output, provider.resource, targets=(target,), auth="oauth",
                    oauth_client_id=target + "-fixture")
    return output / target


def native_command(root, name, binary=None):
    """Hide the entire real home, keeping HOME and CODEX_HOME unchanged."""
    home = root / (name + "-home")
    home.mkdir()
    source = binary or shutil.which(name)
    if not source:
        raise RuntimeError("install the requested native client first")
    executable = root / (name + "-cli")
    shutil.copyfile(Path(source).resolve(), executable)
    executable.chmod(0o700)
    command = ["bwrap", "--ro-bind", "/", "/", "--bind", str(home), str(Path.home()),
               "--bind", str(root), str(root)]
    if name == "codex":
        store = root / "codex-store"
        store.mkdir()
        (store / "config.toml").write_text('mcp_oauth_credentials_store="file"\n')
        current_store = Path(os.environ.get("CODEX_HOME") or str(Path.home() / ".codex")).resolve()
        command += ["--bind", str(store), str(current_store)]
    environment = {key: os.environ[key] for key in ("PATH", "HOME", "CODEX_HOME", "LANG") if key in os.environ}
    environment.update(BROWSER="/bin/true", NO_COLOR="1", DISABLE_TELEMETRY="1",
                       CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1")
    return command + ["--", str(executable)], environment


class AppServer:
    """Native Codex JSON-RPC, without model requests or account credentials."""

    def __init__(self, command, environment, root):
        self.sequence, self.notifications = 0, []
        self.process = subprocess.Popen(command + ["app-server"], cwd=root, env=environment,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.lines = queue.Queue()

        def collect():
            for line in self.process.stdout:
                self.lines.put(line)
            self.lines.put(None)

        threading.Thread(target=collect, daemon=True).start()

    def receive(self, deadline):
        line = self.lines.get(timeout=max(0.1, deadline - time.monotonic()))
        if line is None:
            raise RuntimeError("native Codex app server stopped; child output withheld")
        return json.loads(line)

    def request(self, method, params):
        self.sequence += 1
        identifier = self.sequence
        self.process.stdin.write(json.dumps({"id": identifier, "method": method, "params": params}) + "\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            response = self.receive(deadline)
            if response.get("id") == identifier:
                if "error" in response:
                    # These requests carry fixture-only data; report a method and
                    # numeric code, never raw client diagnostics or credentials.
                    diagnostic = re.sub(r"https?://\S+|[-A-Za-z0-9_./+=~]{16,}", "<redacted>",
                                        str(response["error"].get("message", "")))
                    print("Native app-server step:", method, "; sanitized error:", diagnostic[:600])
                    raise RuntimeError(f"native app-server {method} failed with code {response['error'].get('code')}")
                return response["result"]
            self.notifications.append(response)
        raise RuntimeError("native app-server request timed out")

    def wait_notification(self, method):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            for index, response in enumerate(self.notifications):
                if response.get("method") == method:
                    return self.notifications.pop(index)["params"]
            self.notifications.append(self.receive(deadline))
        raise RuntimeError("native app-server notification timed out")

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.process.stdout.close()


class CodexMCP:
    call = smoke.MCP.call

    def __init__(self, app, server, thread):
        self.app, self.server, self.thread = app, server, thread

    def send(self, method, params):
        assert method == "tools/call"
        return self.app.request("mcpServer/tool/call", {"threadId": self.thread, "server": self.server,
            "tool": params["name"], "arguments": params["arguments"]})


def native_codex(provider, root, binary=None):
    marketplace = fixture_package(root, "codex", provider)
    command, environment = native_command(root, "codex", binary)
    environment.update(CODEX_CA_CERTIFICATE=str(provider.certificate), SSL_CERT_FILE=str(provider.certificate))
    run(command + ["plugin", "marketplace", "add", str(marketplace)], cwd=root, env=environment, timeout=30)
    run(command + ["plugin", "add", "ops-brain-bus@ops-brain-local", "--json"], cwd=root, env=environment, timeout=30)
    app = AppServer(command, environment, root)
    try:
        app.request("initialize", {"clientInfo": {"name": "ops-brain-fixture", "version": "1"},
                                  "capabilities": {"experimentalApi": True}})
        app.process.stdin.write(json.dumps({"method": "initialized"}) + "\n")
        app.process.stdin.flush()
        catalog = app.request("plugin/list", {"cwds": [str(root)]})
        details = app.request("plugin/read", {"pluginName": "ops-brain-bus",
            "marketplacePath": catalog["marketplaces"][0]["path"]})
        assert details["plugin"]["mcpServers"] == ["ops-brain"]
        thread = app.request("thread/start", {"cwd": str(root), "ephemeral": True})["thread"]["id"]
        servers = app.request("mcpServerStatus/list", {"threadId": thread})["data"]
        assert len(servers) == 1 and servers[0]["pluginId"] == "ops-brain-bus@ops-brain-local"
        server = servers[0]["name"]
        login = app.request("mcpServer/oauth/login", {"name": server, "threadId": thread, "timeoutSecs": 30})
        authorization = urlsplit(login["authorizationUrl"])
        assert authorization.scheme == "https" and authorization.netloc == urlsplit(provider.issuer).netloc
        browser = build_opener(HTTPSHandler(context=provider.context))
        with browser.open(login["authorizationUrl"], timeout=10) as response:
            response.read()
        completed = app.wait_notification("mcpServer/oauthLogin/completed")
        assert completed["success"]
        app.request("config/mcpServer/reload", {})
        servers = app.request("mcpServerStatus/list", {"threadId": thread})["data"]
        assert len(servers) == 1 and len(servers[0]["tools"]) == 13
        assert servers[0]["authStatus"] == "oAuth" and not servers[0].get("toolsError")
        exercise(provider, provider.tokens["codex-fixture"], CodexMCP(app, server, thread))
    finally:
        app.close()


def native_claude(provider, root, binary=None):
    import pty  # Only the optional native check needs a Unix terminal.

    marketplace = fixture_package(root, "claude", provider)
    command, environment = native_command(root, "claude", binary)
    environment["NODE_EXTRA_CA_CERTS"] = str(provider.certificate)
    run(command + ["plugin", "marketplace", "add", str(marketplace)], cwd=root, env=environment, timeout=30)
    run(command + ["plugin", "install", "ops-brain-bus@ops-brain-local"], cwd=root, env=environment, timeout=30)
    # Plugin server names are defined by the native client's public contract.
    name = "plugin:ops-brain-bus:ops-brain"
    # The native headless flow requires a TTY. Follow only fixture consent,
    # then paste the resulting loopback redirect into that isolated terminal.
    master, slave = pty.openpty()
    process = subprocess.Popen(command + ["mcp", "login", name, "--no-browser"], cwd=root, env=environment,
        stdin=slave, stdout=slave, stderr=slave)
    os.close(slave)
    output = bytearray()
    try:
        deadline, opened = time.monotonic() + 30, False
        while time.monotonic() < deadline:
            ready, _, _ = select.select([master], [], [], 0.2)
            if ready:
                try:
                    chunk = os.read(master, 4096)
                except OSError:
                    break  # A closed PTY reports EIO on Linux.
                if not chunk:
                    break
                if len(output) < 16384:
                    output.extend(chunk)
            url = terminal_authorization_url(output.decode(errors="replace"), provider.issuer)
            if url and not opened:
                opened = True
                try:
                    provider.opener.open(url, timeout=10)
                    raise RuntimeError("expected fixture consent redirect")
                except HTTPError as response:
                    assert response.code == 302
                    redirect = response.headers["Location"]
                    callback = urlsplit(redirect)
                    assert callback.scheme == "http" and callback.hostname in ("127.0.0.1", "localhost")
                os.write(master, (redirect + "\r").encode())
            if process.poll() is not None:
                break
        if opened and process.poll() is None:
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                pass
        if process.poll() != 0 or not opened:
            diagnostic = re.sub(r"https?://\S+|[-A-Za-z0-9_./+=~]{16,}", "<redacted>",
                                output.decode(errors="replace"))
            print("Native Claude consent URL observed:", opened, "; sanitized error:", diagnostic[:1500])
            raise RuntimeError("native Claude plugin login failed; child output withheld")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
    # Native health-check must use its persisted OAuth grant after a restart.
    before = provider.authenticated_initializations.get("claude-fixture", 0)
    result = run(command + ["mcp", "list"], cwd=root, env=environment, timeout=30)
    assert any(name.encode() in line and b"Connected" in line for line in result.splitlines())
    assert provider.authenticated_initializations.get("claude-fixture", 0) > before
    assert "claude-fixture" in provider.tokens


def terminal_authorization_url(output, issuer):
    # OSC hyperlinks repeat the URL, separated by BEL/ESC rather than spaces.
    # Wait for a terminator so a partial terminal chunk isn't opened as a URL.
    text = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", output)
    match = re.search(r"https://localhost:\d+/authorize\?[^\x00-\x20\x7f]+(?=[\x00-\x20\x7f])", text)
    if not match:
        return None
    if urlsplit(match.group()).netloc != urlsplit(issuer).netloc:
        raise RuntimeError("native client returned an unexpected fixture authorization origin")
    return match.group()


@contextlib.contextmanager
def database(root, start_postgres):
    if not start_postgres:
        url = os.environ.get("OPS_BRAIN_TEST_DATABASE_URL", "")
        if not url or not urlsplit(url).path.endswith("_test"):
            raise RuntimeError("set OPS_BRAIN_TEST_DATABASE_URL to an isolated *_test database, or use --start-postgres")
        yield url
        return
    data, port = root / "postgres", free_port()
    run(["initdb", "-D", str(data), "-U", "ops_brain", "-A", "trust", "--no-locale", "--encoding=UTF8"])
    run(["pg_ctl", "-D", str(data), "-l", str(root / "postgres.log"), "-o",
         f"-F -h 127.0.0.1 -p {port} -k {root}", "-w", "start"])
    try:
        run(["createdb", "-h", "127.0.0.1", "-p", str(port), "-U", "ops_brain", "ops_brain_test"])
        run(["psql", "-h", "127.0.0.1", "-p", str(port), "-U", "ops_brain", "-d", "ops_brain_test",
             "-c", "CREATE EXTENSION vector;"])
        yield f"postgresql://ops_brain@127.0.0.1:{port}/ops_brain_test"
    finally:
        run(["pg_ctl", "-D", str(data), "-m", "fast", "-w", "stop"])


def main():
    global STAGE
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--start-postgres", action="store_true", help="create and remove a temporary local PostgreSQL cluster")
    parser.add_argument("--codex", action="store_true", help="also exercise installed Codex CLI sign-in, with a temporary credential store (Linux + bwrap)")
    parser.add_argument("--codex-binary", type=Path, help="standalone Codex executable, if PATH uses a launcher")
    parser.add_argument("--claude", action="store_true", help="also install and sign in to the Claude Code plugin, using an isolated home (Linux + bwrap)")
    parser.add_argument("--claude-binary", type=Path, help="standalone Claude Code executable, if PATH uses a launcher")
    parser.add_argument("--rust-tests", action="store_true", help="also run cargo test against the isolated database")
    parser.add_argument("--binary", type=Path, default=REPO / "target/debug/ops-brain")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="ops-brain-oauth-") as temporary:
        root = Path(temporary)
        STAGE = "isolated database startup"
        with database(root, args.start_postgres) as url:
            STAGE = "HTTPS test provider startup"
            provider = Provider(root)
            try:
                config = {"issuer": provider.issuer, "jwks_uri": provider.issuer + "/jwks",
                          "jwks_ca_pem": provider.certificate.read_text(), "required_scopes": ["ops-brain"],
                          "bindings": [{"subject": "fixture-user", "oauth_client_id": client,
                                        "from_agent": agent, "exclude_client_data": restricted}
                                       for client, agent, restricted in (("claude-fixture", "Fixture-Claude", False),
                                                                         ("codex-fixture", "Fixture-Codex", True))]}
                environment = {"PATH": os.environ["PATH"], "RUST_LOG": "warn", "DATABASE_URL": url,
                    "OPS_BRAIN_TRANSPORT": "http", "OPS_BRAIN_LISTEN": urlsplit(provider.proxy).netloc,
                    "OPS_BRAIN_AUTH_TOKEN": secrets.token_urlsafe(48), "OPS_BRAIN_EMBEDDINGS_ENABLED": "false",
                    "OPS_BRAIN_ALLOWED_HOSTS": urlsplit(provider.issuer).netloc,
                    "OPS_BRAIN_OAUTH_CONFIG": json.dumps(config), "OPS_BRAIN_OAUTH_RESOURCE_URL": provider.resource}
                with (root / "server.log").open("wb") as log:
                    STAGE = "ops-brain startup/readiness"
                    process = subprocess.Popen([str(args.binary.resolve())], cwd=root, env=environment, stdout=log, stderr=log)
                    try:
                        for _ in range(100):
                            if process.poll() is not None:
                                raise RuntimeError("fixture ops-brain stopped before readiness; output withheld")
                            try:
                                with provider.opener.open(provider.issuer + "/ready", timeout=1) as response:
                                    if response.status == 200:
                                        break
                            except (OSError, URLError):
                                time.sleep(0.1)
                        else:
                            raise RuntimeError("fixture ops-brain readiness timed out")
                        with provider.opener.open(provider.issuer + "/.well-known/oauth-protected-resource/mcp", timeout=10) as response:
                            assert json.load(response)["resource"] == provider.resource
                        STAGE = "PKCE sign-in"
                        token = provider.authorize("codex-fixture")
                        STAGE = "OAuth MCP handoff round trip"
                        exercise(provider, token)
                        print("PASS: HTTPS discovery, PKCE, code replay rejection, distinct seats, handoff/claim/completion/thread, expiry/audience and endpoint restrictions")
                        if args.rust_tests:
                            STAGE = "Rust integration tests"
                            test_environment = dict(environment, HOME=str(Path.home()), OPS_BRAIN_AGENT_TOKENS="",
                                                    OPS_BRAIN_MACHINE_TOKENS="", OPS_BRAIN_INSTRUCTIONS_EXTRA="")
                            results = run(["cargo", "test", "--offline"], cwd=REPO, env=test_environment, timeout=600)
                            for summary in re.findall(rb"test result: [^\n]+", results):
                                print(summary.decode())
                        if args.codex:
                            STAGE = "native Codex plugin install/sign-in/tool calls"
                            native_codex(provider, root, args.codex_binary)
                            print("PASS: installed Codex plugin, native OAuth sign-in, 13 tools and native handoff/thread/identity checks")
                        if args.claude:
                            STAGE = "native Claude Code plugin install/sign-in"
                            native_claude(provider, root, args.claude_binary)
                            print("PASS: installed Claude Code plugin, native OAuth sign-in and persisted-grant connection check")
                    finally:
                        process.terminate()
                        process.wait(timeout=10)
            finally:
                provider.close()


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        location = traceback.extract_tb(error.__traceback__)[-1]
        if isinstance(error, RuntimeError):
            diagnostic = re.sub(r"https?://\S+|[-A-Za-z0-9_./+=~]{16,}", "<redacted>", str(error))
            print("Fixture diagnostic:", diagnostic)
        sys.exit(f"OAuth fixture failed during {STAGE} ({location.name}:{location.lineno}); child output and credentials were withheld.")
