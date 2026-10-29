#!/usr/bin/env python3
"""Build self-hosted client plugin folders and ZIPs without reading credentials."""

import argparse
import copy
import json
from pathlib import Path
import shutil
import sys
from urllib.parse import urlsplit
import zipfile


REPO = Path(__file__).resolve().parents[1]
SOURCE = REPO / "plugins" / "ops-brain-bus"
TARGETS = ("claude", "codex", "grok", "cursor")
SUBMISSION_TARGET = "openai"
NAME = "ops-brain-bus"


def read_file(root, relative):
    path = root / relative
    # Only ship the explicit files below, never a working tree or private config.
    if path.resolve() != path.absolute() or not path.is_file():
        raise ValueError(f"Package input must be a regular, non-symlink file: {relative}")
    return path.read_bytes()


def json_bytes(value):
    return (json.dumps(value, indent=2, ensure_ascii=False) + "\n").encode()


def valid_oauth_scope(value):
    # Match the server's RFC 6749 scope-token grammar, including punctuation.
    return isinstance(value, str) and bool(value) and all(
        character == "!" or "#" <= character <= "[" or "]" <= character <= "~" for character in value)


def package_files(target, url, source=SOURCE, repo=REPO, auth="token", oauth_client_id=None,
                  oauth_callback_port=None, oauth_scopes=None):
    if target not in (*TARGETS, SUBMISSION_TARGET):
        raise ValueError("Unknown plugin target")
    if auth not in ("token", "oauth"):
        raise ValueError("Unknown plugin authentication mode")
    if target == SUBMISSION_TARGET and auth != "oauth":
        raise ValueError("The OpenAI submission package requires --auth oauth")
    if oauth_client_id is not None:
        if auth != "oauth" or target not in ("claude", "codex", "grok", SUBMISSION_TARGET):
            raise ValueError("A provided OAuth client ID requires --auth oauth and one Claude, Codex, Grok or OpenAI target")
        if not oauth_client_id or len(oauth_client_id) > 2048 or any(
                character.isspace() or ord(character) < 32 for character in oauth_client_id):
            raise ValueError("Invalid OAuth client ID")
    if oauth_callback_port is not None or oauth_scopes is not None:
        if target != "grok" or auth != "oauth" or oauth_client_id is None:
            raise ValueError("OAuth callback port and scopes require one Grok OAuth target with a public client ID")
    if oauth_callback_port is not None and (
            type(oauth_callback_port) is not int or not 1 <= oauth_callback_port <= 65535):
        raise ValueError("Invalid OAuth callback port")
    if oauth_scopes is not None and (not isinstance(oauth_scopes, list) or not oauth_scopes
            or len(oauth_scopes) > 20 or any(not valid_oauth_scope(scope) or len(scope) > 2048
                for scope in oauth_scopes)):
        raise ValueError("Invalid OAuth scopes")
    try:
        parsed = urlsplit(url)
        hostname = parsed.hostname
        parsed.port  # Reject malformed ports without echoing the authority.
    except ValueError:
        raise ValueError("Invalid MCP URL") from None
    if (parsed.scheme not in ("http", "https") or not hostname
            or parsed.username or parsed.password or parsed.query or parsed.fragment
            or any(character.isspace() or ord(character) < 32 for character in url)
            or (parsed.scheme == "http" and hostname not in ("localhost", "127.0.0.1", "::1"))):
        raise ValueError("Use an HTTPS MCP URL without credentials, query, fragment or whitespace (HTTP is loopback-only)")
    metadata = json.loads(read_file(source, ".cursor-plugin/plugin.json"))
    # Select fields rather than leaking client-specific options into other manifests.
    common = {key: metadata[key] for key in (
        "name", "version", "description", "author", "homepage", "repository", "license", "keywords"
    )}
    if common["name"] != NAME:
        raise ValueError("Unexpected plugin name")
    files = {path: read_file(source, path) for path in (
        "README.md", "assets/logo.svg", "skills/ops-brain-bus/SKILL.md"
    )}
    for license_name in ("LICENSE-MIT", "LICENSE-APACHE"):
        files[license_name] = read_file(repo, license_name)
    mcp = json.loads(read_file(source, "mcp.json"))
    # Keep placeholders literal. Building a distributable must never resolve the
    # builder's environment, particularly OPS_BRAIN_AGENT_TOKEN.
    server = mcp.get("mcpServers", {}).get("ops-brain")
    expected = {"type": "http", "url": "${OPS_BRAIN_URL}",
                "headers": {"Authorization": "Bearer ${OPS_BRAIN_AGENT_TOKEN}"}}
    if server != expected or set(mcp) != {"mcpServers"} or len(mcp["mcpServers"]) != 1:
        raise ValueError("MCP source must contain only the documented URL and token placeholders")
    # Pin the non-secret endpoint at build time: clients differ in whether URL
    # environment expansion is supported. Credentials always stay unresolved.
    server["url"] = url
    if auth == "oauth":
        if parsed.scheme != "https":
            raise ValueError("OAuth packages require an HTTPS resource URL")
        server.pop("headers")
    if target == "cursor":
        if auth == "oauth":
            metadata.pop("variables")
        else:
            metadata["variables"]["properties"].pop("OPS_BRAIN_URL")
            metadata["variables"]["required"].remove("OPS_BRAIN_URL")
        files[".cursor-plugin/plugin.json"] = json_bytes(metadata)
        files["mcp.json"] = json_bytes(mcp)
    elif target in ("claude", "grok"):
        manifest = dict(common, skills="./skills/", mcpServers="./.mcp.json")
        if target == "claude":
            manifest["privacyPolicyUrl"] = metadata["homepage"] + "#privacy-and-data-handling"
        files[f".{target}-plugin/plugin.json"] = json_bytes(manifest)
        if oauth_client_id is not None:
            server["oauth"] = {"clientId": oauth_client_id}
            if oauth_callback_port is not None:
                server["oauth"]["callbackPort"] = oauth_callback_port
            if oauth_scopes is not None:
                server["oauth"]["scopes"] = oauth_scopes
        files[".mcp.json"] = json_bytes(mcp)
    else:
        manifest = dict(common, skills="./skills/", mcpServers="./.mcp.json",
                        interface={"displayName": metadata["displayName"],
                                   "shortDescription": "Handoffs for your AI team",
                                   "longDescription": metadata["description"],
                                   "privacyPolicyURL": metadata["homepage"] + "#privacy-and-data-handling",
                                   "logo": "./assets/logo.svg"})
        if target == "codex":
            files[".codex-plugin/plugin.json"] = json_bytes(manifest)
        codex_mcp = copy.deepcopy(mcp)
        codex_server = codex_mcp["mcpServers"]["ops-brain"]
        if auth == "oauth":
            oauth = {"type": "oauth"}
            if oauth_client_id is not None:
                oauth["client"] = {"mode": "provided", "clientId": oauth_client_id,
                                   "tokenEndpointAuthMethod": "none"}
                if target == "codex":
                    # Installed Codex uses its native config key; the OpenAI
                    # extension declares the public submission contract.
                    codex_server["oauth"] = {"client_id": oauth_client_id}
            codex_server["extensions"] = {"com.openai": {"auth": oauth}}
            if target == SUBMISSION_TARGET:
                # Submission follows the published portable contract. Keep it
                # separate: Codex 0.165.0 skips bundled MCP servers when a root
                # plugin.json is present, even though the plugin is installed.
                portable = dict(common)
                portable["$schema"] = "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json"
                portable["extensions"] = {"com.openai": {"interface": manifest["interface"]}}
                files["plugin.json"] = json_bytes(portable)
                codex_mcp["$schema"] = "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json"
                codex_server["type"] = "streamable-http"
        else:
            codex_server.pop("headers")
            codex_server["bearer_token_env_var"] = "OPS_BRAIN_AGENT_TOKEN"
        files["mcp.json" if target == SUBMISSION_TARGET else ".mcp.json"] = json_bytes(codex_mcp)
    return files


def build(output, url, targets=TARGETS, source=SOURCE, repo=REPO, auth="token", oauth_client_id=None,
          oauth_callback_port=None, oauth_scopes=None):
    output = Path(output)
    # Plan every input before creating output, including rejection of a credential
    # accidentally substituted into source MCP configuration.
    if oauth_client_id is not None and len(targets) != 1:
        raise ValueError("Build one target at a time when providing an OAuth client ID")
    planned = [(target, package_files(target, url, source, repo, auth, oauth_client_id,
                                     oauth_callback_port, oauth_scopes)) for target in targets]
    # A new directory makes accidental overwrites (including existing credentials)
    # impossible, and keeps archives out of the source plugin.
    if output.resolve() == source.resolve() or source.resolve() in output.resolve().parents:
        raise ValueError("Output must be outside the source plugin")
    output.mkdir(parents=True, exist_ok=False)
    try:
        for target, files in planned:
            root = output / target / NAME
            for relative, data in sorted(files.items()):
                destination = root / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(data)
            if target == SUBMISSION_TARGET:
                catalog_path = None  # Public submission artifact, not a local marketplace.
            elif target == "codex":
                catalog_path = output / target / ".agents/plugins/marketplace.json"
                catalog = {"name": "ops-brain-local",
                           "interface": {"displayName": "ops-brain self-hosted"},
                           "plugins": [{"name": NAME,
                                        "source": {"source": "local", "path": f"./{NAME}"},
                                        "policy": {"installation": "AVAILABLE", "authentication": "ON_USE"},
                                        "category": "Productivity"}]}
            else:
                catalog_path = output / target / f".{target}-plugin/marketplace.json"
                catalog = {"name": "ops-brain-local",
                           "owner": json.loads(files[f".{target}-plugin/plugin.json"])["author"],
                           "plugins": [{"name": NAME, "source": f"./{NAME}"}]}
                description = "Connect your agents to your self-hosted ops-brain handoff bus"
                if target == "grok":
                    catalog["description"] = description
                else:
                    catalog["metadata"] = {"description": description}
            if catalog_path is not None:
                catalog_path.parent.mkdir(parents=True, exist_ok=True)
                catalog_path.write_bytes(json_bytes(catalog))
            archive = output / f"{NAME}-{target}.zip"
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as bundle:
                for relative, data in sorted(files.items()):
                    entry = zipfile.ZipInfo(f"{NAME}/{relative}", date_time=(2020, 1, 1, 0, 0, 0))
                    entry.compress_type = zipfile.ZIP_DEFLATED
                    entry.external_attr = 0o100644 << 16
                    bundle.writestr(entry, data)
    except BaseException:
        shutil.rmtree(output)
        raise
    return [output / f"{NAME}-{target}.zip" for target in targets]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="new directory for folders and ZIPs")
    parser.add_argument("--url", required=True, help="your non-secret MCP endpoint; embedded in the packages")
    parser.add_argument("--target", choices=(*TARGETS, SUBMISSION_TARGET, "all"), default="all",
                        help="client for local installation; openai is a separate portable OAuth submission artifact")
    parser.add_argument("--auth", choices=("token", "oauth"), default="token",
                        help="fixed per-agent token (default), or external OAuth sign-in")
    parser.add_argument("--oauth-client-id", help="public, pre-registered PKCE client ID (one Claude, Codex, Grok or OpenAI target only; never a secret)")
    parser.add_argument("--oauth-callback-port", type=int, help="fixed loopback callback port for a Grok public OAuth client")
    parser.add_argument("--oauth-scope", action="append", dest="oauth_scopes",
                        help="scope requested by a Grok public OAuth client; repeat for multiple scopes")
    args = parser.parse_args()
    try:
        archives = build(args.output, args.url, TARGETS if args.target == "all" else (args.target,),
                         auth=args.auth, oauth_client_id=args.oauth_client_id,
                         oauth_callback_port=args.oauth_callback_port, oauth_scopes=args.oauth_scopes)
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f"Plugin packaging failed: {error}\n")
    for archive in archives:
        print(f"Built {archive}")
    print(f"Built for {args.auth} authentication; package validation does not establish public directory approval.")


if __name__ == "__main__":
    sys.exit(main())
