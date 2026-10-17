#!/usr/bin/env python3
"""Write private, operator-owned deployment files without printing credentials."""

import argparse
import json
import os
from pathlib import Path
import re
import secrets
import sys


def agent_slug(value):
    if value in (".", "..") or not re.fullmatch(r"[A-Za-z0-9._-]{1,80}", value):
        raise argparse.ArgumentTypeError("agent must be a 1–80 character slug")
    return value


def initialize(directory, agents, image_tag):
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", image_tag):
        raise ValueError("image tag must be an explicit release such as v6.1.1")
    if not agents or len(set(a.lower() for a in agents)) != len(agents):
        raise ValueError("supply distinct agent names (case insensitive)")
    for agent in agents:
        agent_slug(agent)
    directory = Path(directory)
    if directory.is_symlink():
        raise ValueError("destination must not be a symlink")
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    env_path = directory / ".env"
    token_directory = directory / "agent-tokens"
    if env_path.exists() or env_path.is_symlink() or token_directory.exists() or token_directory.is_symlink():
        raise ValueError(".env or agent-tokens already exists; refusing to replace credentials")
    token_directory.mkdir(mode=0o700)
    created = []
    try:
        bindings = []
        for agent in agents:
            token = secrets.token_hex(32)
            path = token_directory / agent
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            created.append(path)
            with os.fdopen(fd, "w") as output:
                output.write(token)
            bindings.append({"token": token, "from_agent": agent})
        fd = os.open(env_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        created.append(env_path)
        with os.fdopen(fd, "w") as output:
            output.write(
                f"OPS_BRAIN_IMAGE=ghcr.io/thek3nsai/ops-brain:{image_tag}\n"
                f"POSTGRES_PASSWORD={secrets.token_hex(32)}\n"
                f"OPS_BRAIN_AUTH_TOKEN={secrets.token_hex(32)}\n"
                "OPS_BRAIN_AGENT_TOKENS='" + json.dumps(bindings, separators=(",", ":")) + "'\n"
                "OPS_BRAIN_EMBEDDINGS_ENABLED=false\n"
                "OPS_BRAIN_BIND=127.0.0.1\n"
                "OPS_BRAIN_PORT=3000\n"
            )
    except BaseException:
        for path in reversed(created):
            path.unlink(missing_ok=True)
        token_directory.rmdir()
        raise
    return env_path, token_directory


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--agent", action="append", type=agent_slug, required=True)
    parser.add_argument("--image-tag", required=True, help="an existing published release, e.g. v6.1.1")
    args = parser.parse_args()
    try:
        env_path, token_directory = initialize(args.directory, args.agent, args.image_tag)
    except (OSError, ValueError) as error:
        parser.exit(1, f"Setup failed: {error}\n")
    print(f"Created {env_path} and {token_directory} (credentials were not printed).")
    print("Keep .env on the server; deliver each agent only its own token file.")


if __name__ == "__main__":
    sys.exit(main())
