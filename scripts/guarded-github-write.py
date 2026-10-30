#!/usr/bin/env python3
"""Scan an explicit GitHub REST payload before publishing it with gh.

Usage: python3 scripts/guarded-github-write.py POST repos/OWNER/REPO/pulls --input request.json

Supports PRs, issues, their comments/reviews and releases. This is an opt-in
publishing path, not an interceptor for direct gh, MCP or browser writes.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / ".github" / "scripts"))
import fleet_string_guard as guard


# Deliberately bounded REST routes: no GraphQL, query strings, alternate hosts,
# generated release notes or interactive/editor defaults that were not scanned.
ROUTES = {
    "POST": [r"pulls", r"issues", r"issues/\d+/comments", r"pulls/\d+/reviews", r"pulls/\d+/comments",
             r"pulls/\d+/comments/\d+/replies", r"releases"],
    "PATCH": [r"pulls/\d+", r"issues/\d+", r"issues/comments/\d+",
              r"pulls/comments/\d+", r"releases/\d+"],
    "PUT": [r"pulls/\d+/reviews/\d+"],
}
REPOSITORY = r"repos/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/"


class QuietParser(argparse.ArgumentParser):
    def error(self, message):
        # argparse's default error includes unrecognized arguments verbatim.
        self.exit(2, "GitHub write refused: invalid arguments; use --help.\n")


def json_strings(value):
    """Scan decoded strings, including nested inline review comments and keys."""
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for key, child in value.items():
            yield key
            yield from json_strings(child)
    elif isinstance(value, list):
        for child in value:
            yield from json_strings(child)


def prepare(method: str, endpoint: str, raw: bytes) -> bytes:
    if any(part in {".", ".."} for part in endpoint.split("/")) or not any(re.fullmatch(REPOSITORY + route, endpoint)
               for route in ROUTES.get(method, [])):
        raise ValueError("unsupported method or endpoint")
    payload = json.loads(raw)
    if not isinstance(payload, dict):
        raise ValueError("request must be a JSON object")
    if payload.get("generate_release_notes"):
        raise ValueError("generated release notes cannot be scanned before publication")
    if method == "PATCH" and re.fullmatch(REPOSITORY + r"releases/\d+", endpoint) and "draft" in payload:
        raise ValueError("release draft transitions can publish stored text that was not scanned")
    # All supported writes carry explicit body text. Empty is fine; omitting
    # it risks publishing a pre-existing draft or server-generated defaults.
    if not isinstance(payload.get("body"), str):
        raise ValueError("request requires an explicit string body")
    sources = [("endpoint", endpoint.encode("utf-8"))]
    sources.extend(("payload", value.encode("utf-8")) for value in json_strings(payload))
    if guard.scan_sources(sources):
        raise ValueError("request blocked by the fleet string guard")
    # Snapshot once: scan and send the same decoded object. gh never reopens
    # the source file, so an editor cannot change it between scan and send.
    return json.dumps(payload, ensure_ascii=False).encode("utf-8")


def main() -> int:
    parser = QuietParser(description=__doc__)
    parser.add_argument("method", help="POST, PATCH or PUT")
    parser.add_argument("endpoint", help="repos/OWNER/REPO/... (supported public-text routes only)")
    parser.add_argument("--input", required=True, help="JSON request file, or - for stdin")
    parser.add_argument("--dry-run", action="store_true", help="scan without publishing")
    args = parser.parse_args()
    try:
        raw = sys.stdin.buffer.read() if args.input == "-" else Path(args.input).read_bytes()
        data = prepare(args.method, args.endpoint, raw)
    except (OSError, ValueError, UnicodeError, RecursionError):
        # Paths, parser errors and payloads may themselves carry private text.
        print("GitHub write refused: invalid, unreadable, unsupported or guarded request.", file=sys.stderr)
        return 1
    if args.dry_run:
        print("Dry run: request scanned; nothing published.")
        return 0
    try:
        result = subprocess.run(
            ["gh", "api", "--method", args.method, args.endpoint, "--input", "-", "--silent"],
            input=data, capture_output=True,
        )
    except OSError:
        print("GitHub write failed: gh could not run.", file=sys.stderr)
        return 1
    # gh/server error output can include supplied text. Keep it out of logs.
    if result.returncode:
        print(f"GitHub write failed: gh exited {result.returncode}.", file=sys.stderr)
        return 1
    print("GitHub write completed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
