#!/usr/bin/env python3
"""Local fixture tests; no deployment, server, credentials or external services."""

import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


setup = load("setup-self-host")
smoke = load("mcp-smoke")
packaging = load("package-plugin")
oauth_fixture = load("test-oauth-flow")


class TerminalTests(unittest.TestCase):
    def test_claude_hyperlinked_consent_stops_at_controls_and_rejects_other_origins(self):
        issuer = "https://localhost:12345"
        url = issuer + "/authorize?client_id=fixture&state=fixture-state"
        output = "\x1b]8;;" + url + "\x07" + url + "\x1b]8;;\x07\r\n"
        self.assertEqual(oauth_fixture.terminal_authorization_url(output, issuer), url)
        self.assertIsNone(oauth_fixture.terminal_authorization_url(url, issuer))
        with self.assertRaises(RuntimeError):
            oauth_fixture.terminal_authorization_url(output, "https://localhost:54321")


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
            b'event: message\ndata:\n\n'
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


class PackagingTests(unittest.TestCase):
    def test_oauth_archives_declare_sign_in_without_fixed_credential_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            archives = packaging.build(Path(temporary) / "output", "https://bus.example.com/mcp", auth="oauth")
            for target, archive in zip(packaging.TARGETS, archives):
                with zipfile.ZipFile(archive) as bundle:
                    config_name = "mcp.json" if target == "cursor" else ".mcp.json"
                    server = json.loads(bundle.read(f"ops-brain-bus/{config_name}"))["mcpServers"]["ops-brain"]
                    self.assertNotIn("headers", server)
                    self.assertNotIn("bearer_token_env_var", server)
                    self.assertEqual(server["url"], "https://bus.example.com/mcp")
                    if target == "codex":
                        self.assertEqual(server["extensions"]["com.openai"]["auth"]["type"], "oauth")
                        self.assertNotIn("ops-brain-bus/plugin.json", bundle.namelist())
                    if target == "cursor":
                        metadata = json.loads(bundle.read("ops-brain-bus/.cursor-plugin/plugin.json"))
                        self.assertNotIn("variables", metadata)
            with self.assertRaises(ValueError):
                packaging.build(Path(temporary) / "plaintext", "http://127.0.0.1:3000/mcp", auth="oauth")

    def test_openai_submission_has_portable_components_without_local_credential_config(self):
        with tempfile.TemporaryDirectory() as temporary:
            archive, = packaging.build(Path(temporary) / "submission", "https://bus.example.com/mcp",
                targets=("openai",), auth="oauth", oauth_client_id="public-client")
            with zipfile.ZipFile(archive) as bundle:
                portable = json.loads(bundle.read("ops-brain-bus/plugin.json"))
                server = json.loads(bundle.read("ops-brain-bus/mcp.json"))["mcpServers"]["ops-brain"]
                self.assertEqual(portable["name"], "ops-brain-bus")
                self.assertEqual(server["type"], "streamable-http")
                self.assertEqual(server["extensions"]["com.openai"]["auth"]["client"]["clientId"], "public-client")
                self.assertNotIn("headers", server)
                self.assertNotIn("bearer_token_env_var", server)
                self.assertNotIn("ops-brain-bus/.mcp.json", bundle.namelist())
                self.assertNotIn("ops-brain-bus/.codex-plugin/plugin.json", bundle.namelist())
            self.assertFalse((Path(temporary) / "submission/openai/.agents").exists())
            with self.assertRaises(ValueError):
                packaging.build(Path(temporary) / "token", "https://bus.example.com/mcp", targets=("openai",))

    def test_provided_oauth_clients_are_public_target_specific_and_require_oauth(self):
        with tempfile.TemporaryDirectory() as temporary:
            for target in ("claude", "codex"):
                archive, = packaging.build(Path(temporary) / target, "https://bus.example.com/mcp",
                    targets=(target,), auth="oauth", oauth_client_id=f"{target}-public-client")
                with zipfile.ZipFile(archive) as bundle:
                    server = json.loads(bundle.read("ops-brain-bus/.mcp.json"))["mcpServers"]["ops-brain"]
                    if target == "claude":
                        self.assertEqual(server["oauth"]["clientId"], "claude-public-client")
                    else:
                        self.assertEqual(server["oauth"]["client_id"], "codex-public-client")
                        self.assertEqual(server["extensions"]["com.openai"]["auth"]["client"],
                            {"mode": "provided", "clientId": "codex-public-client", "tokenEndpointAuthMethod": "none"})
            for options in ({}, {"auth": "oauth"}, {"auth": "oauth", "targets": ("grok",)},
                            {"auth": "oauth", "targets": ("cursor",)}, {"targets": ("codex",)},
                            {"auth": "oauth", "targets": ("codex",), "oauth_client_id": "bad\nclient"}):
                output = Path(temporary) / "invalid"
                with self.assertRaises(ValueError):
                    packaging.build(output, "https://bus.example.com/mcp", **dict(
                        {"oauth_client_id": "public-client"}, **options))
                self.assertFalse(output.exists())

    def test_malformed_url_errors_do_not_echo_credentials(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "output"
            result = subprocess.run(
                [sys.executable, str(packaging.REPO / "scripts/package-plugin.py"),
                 "--url", "https://fixture-user:fixture-password@℀.example/mcp",
                 "--output", str(output)], capture_output=True, text=True, check=False)
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("fixture-user", result.stdout + result.stderr)
            self.assertNotIn("fixture-password", result.stdout + result.stderr)
            self.assertIn("Invalid MCP URL", result.stderr)
            self.assertFalse(output.exists())

    def test_archives_share_workflow_and_never_read_environment_credentials(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "source"
            shutil.copytree(packaging.SOURCE, source)
            (source / ".env").write_text("DO_NOT_SHIP=private-fixture-data")
            (source / "assets" / "private-file").write_text("private-fixture-data")
            with patch.dict(os.environ, {"OPS_BRAIN_AGENT_TOKEN": "private-fixture-token",
                                         "OPS_BRAIN_URL": "https://private.example.com/mcp"}):
                archives = packaging.build(Path(temporary) / "output", "https://bus.example.com/mcp", source=source)
            for target, archive in zip(packaging.TARGETS, archives):
                with zipfile.ZipFile(archive) as bundle:
                    contents = {name: bundle.read(name) for name in bundle.namelist()}
                combined = b"\n".join(contents.values())
                self.assertNotIn(b"private-fixture", combined)
                self.assertNotIn(b"private.example.com", combined)
                self.assertEqual(contents["ops-brain-bus/skills/ops-brain-bus/SKILL.md"],
                                 (source / "skills/ops-brain-bus/SKILL.md").read_bytes())
                self.assertIn("ops-brain-bus/LICENSE-MIT", contents)
                self.assertIn("ops-brain-bus/LICENSE-APACHE", contents)
                config_name = "mcp.json" if target == "cursor" else ".mcp.json"
                mcp = json.loads(contents[f"ops-brain-bus/{config_name}"])["mcpServers"]["ops-brain"]
                self.assertEqual(mcp["url"], "https://bus.example.com/mcp")
                if target == "codex":
                    self.assertEqual(mcp["bearer_token_env_var"], "OPS_BRAIN_AGENT_TOKEN")
                    self.assertNotIn("headers", mcp)
                else:
                    self.assertEqual(mcp["headers"]["Authorization"], "Bearer ${OPS_BRAIN_AGENT_TOKEN}")

    def test_existing_output_preserved_and_repeated_builds_are_identical(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "first"
            first = packaging.build(output, "https://bus.example.com/mcp")
            original = [path.read_bytes() for path in first]
            with self.assertRaises(FileExistsError):
                packaging.build(output, "https://bus.example.com/mcp")
            second = packaging.build(Path(temporary) / "second", "https://bus.example.com/mcp")
            self.assertEqual(original, [path.read_bytes() for path in first])
            self.assertEqual(original, [path.read_bytes() for path in second])

    def test_private_url_or_substituted_token_or_symlink_rejected_before_writing(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "output"
            for url in ("http://example.com/mcp", "https://user:secret@example.com/mcp",
                        "https://example.com/mcp?token=secret", "https://example.com/mcp#secret",
                        "https://example.com:invalid/mcp", "https://example.com/mcp\n"):
                with self.assertRaises(ValueError):
                    packaging.build(output, url)
                self.assertFalse(output.exists())
            source = Path(temporary) / "source"
            shutil.copytree(packaging.SOURCE, source)
            mcp = source / "mcp.json"
            original = mcp.read_text()
            mcp.write_text(original.replace("${OPS_BRAIN_AGENT_TOKEN}", "private-fixture-token"))
            with self.assertRaises(ValueError):
                packaging.build(output, "https://bus.example.com/mcp", source=source)
            self.assertFalse(output.exists())
            mcp.write_text(original)
            logo = source / "assets/logo.svg"
            logo.unlink()
            logo.symlink_to(packaging.SOURCE / "assets/logo.svg")
            with self.assertRaises(ValueError):
                packaging.build(output, "https://bus.example.com/mcp", source=source)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
