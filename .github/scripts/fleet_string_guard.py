#!/usr/bin/env python3
"""Fleet-private string guard — hash-based, full-tree.

Why hashes instead of a literal pattern: this repo is public, and a guard that
enumerates the strings it protects is the highest-signal place to look for
exactly those values. The denylist ships as salted SHA-256 digests
(`.github/fleet-denylist.sha256`), so reading the guard tells you nothing you
did not already have. Someone who already knows a value can confirm it; nobody
learns one. That is the correct level — the guarded strings are treated as
already disclosed (they sit in this repo's history since 2026-04-26), so the
goal is to stop *handing over* the list, not to keep a secret that no longer is.

Why not a repository secret: this is a public repo, and `pull_request` runs from
forks receive no secrets. A secret-sourced denylist is therefore either blind or
red on every fork PR. Hashes behave identically in both cases and need no
provisioning.

Scans every tracked file, not just added diff lines — a delta-only guard
certifies a dirty tree forever. Matching is case-insensitive and finds a
guarded value anywhere inside a longer identifier, not only on separator
boundaries.

Deliberate non-goals, so nobody over-trusts this: it does not decode base64,
percent-encoding, or any other transformation, and it will not match a value
whose characters have been re-spaced or split across lines. It catches
accidents, not an adversary who is trying to smuggle a string past it.

Disclosure note: the denylist publishes each guarded value's *length*. That is
deliberate and is strictly less than the previous design leaked (the whole
plaintext), and less than a masked-output scheme leaks (length plus first and
last character). Match output names a digest prefix and no plaintext at all.
"""

from __future__ import annotations

import hashlib
import os
import re
import subprocess
import sys
from pathlib import Path

# Overridable so the test suite can drive the guard against a synthetic repo
# and a synthetic denylist — the tests must never need a real guarded value.
REPO_ROOT = Path(os.environ.get("FLEET_GUARD_ROOT") or Path(__file__).resolve().parents[2])
DENYLIST = Path(
    os.environ.get("FLEET_GUARD_DENYLIST") or REPO_ROOT / ".github" / "fleet-denylist.sha256"
)
ALLOWLIST = Path(
    os.environ.get("FLEET_GUARD_ALLOWLIST") or REPO_ROOT / ".github" / "fleet-denylist-allow.txt"
)

# Repo-visible, fixed. Not a secret — it exists so a digest here cannot be
# resolved against a precomputed public rainbow table, only against a candidate
# list an attacker builds on purpose.
SALT = b"ops-brain-fleet-guard-v1:"

# How many guarded classes the denylist must contain. Pinned so that a change
# which drops one is a red build rather than a quietly narrower guard.
EXPECTED_CLASSES = int(os.environ.get("FLEET_GUARD_EXPECTED_CLASSES") or 3)

ENTRY_RE = re.compile(r"^(\d{1,3}):([0-9a-f]{64})$")

# Maximal runs of identifier-ish bytes. A guarded value is a hostname, so it
# cannot span a space or a newline; restricting windows to these runs is what
# keeps a full-tree byte scan linear and fast.
RUN_RE = re.compile(rb"[A-Za-z0-9._:/@+-]+")

# Generous: the whole tracked tree is ~1 MB. Anything over this is reported as
# unscanned rather than skipped in silence.
MAX_BYTES = int(os.environ.get("FLEET_GUARD_MAX_BYTES") or 16 * 1024 * 1024)


def digest(token: bytes) -> str:
    return hashlib.sha256(SALT + token.lower()).hexdigest()


def load_denylist() -> dict[int, set[str]]:
    """Digests grouped by the byte length of the value they cover.

    Entries are validated strictly. An unvalidated denylist is the worst
    failure mode this script has: a truncated paste or a stray `\\r` would
    disable a class while the run still reported a clean tree.
    """
    if not DENYLIST.is_file():
        sys.exit(f"::error::denylist missing: {DENYLIST}")
    by_len: dict[int, set[str]] = {}
    count = 0
    for lineno, raw in enumerate(DENYLIST.read_text().splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        m = ENTRY_RE.fullmatch(line.lower())
        if not m:
            sys.exit(
                f"::error::denylist {DENYLIST.name}:{lineno}: malformed entry "
                f"(want `<length>:<64 hex chars>`)"
            )
        length = int(m.group(1))
        if length < 3:
            sys.exit(f"::error::denylist {DENYLIST.name}:{lineno}: length {length} is too short")
        by_len.setdefault(length, set()).add(m.group(2))
        count += 1
    if count != EXPECTED_CLASSES:
        sys.exit(
            f"::error::denylist has {count} entries, expected {EXPECTED_CLASSES}. "
            f"If this is intentional, update EXPECTED_CLASSES in {Path(__file__).name}."
        )
    return by_len


def load_allowlist() -> set[str]:
    if not ALLOWLIST.is_file():
        return set()
    out = set()
    for raw in ALLOWLIST.read_text().splitlines():
        line = raw.split("#", 1)[0].strip()
        if line:
            out.add(line)
    return out


def tracked_files() -> list[str]:
    res = subprocess.run(
        ["git", "-C", str(REPO_ROOT), "ls-files", "-z"],
        check=True,
        capture_output=True,
    )
    return [p.decode() for p in res.stdout.split(b"\0") if p]


def scan_bytes(data: bytes, by_len: dict[int, set[str]]) -> list[tuple[int, str]]:
    """(line number, digest prefix) for every guarded value present in `data`."""
    hits = []
    for lineno, line in enumerate(data.split(b"\n"), 1):
        low = line.lower()
        for run in RUN_RE.finditer(low):
            seg = run.group()
            n = len(seg)
            for length, digests in by_len.items():
                for start in range(0, n - length + 1):
                    d = digest(seg[start : start + length])
                    if d in digests:
                        hits.append((lineno, d[:8]))
    return hits


def scan_file(path: str, by_len: dict[int, set[str]], unscanned: list[tuple[str, str]]):
    full = REPO_ROOT / path
    try:
        # Symlinks are not followed and submodule gitlinks are not entered:
        # `git ls-files` reports the link/gitlink itself, and its target is
        # covered by its own repo. Deliberate, not incidental.
        if full.is_symlink():
            return []
        if not full.is_file():
            return []
        if full.stat().st_size > MAX_BYTES:
            unscanned.append((path, f"over the {MAX_BYTES // (1024 * 1024)} MB size cap"))
            return []
        data = full.read_bytes()
    except OSError as exc:
        # Never swallow this — an unreadable file contributing zero hits to a
        # run that then reports "Clean" is the same fail-open as skipping it.
        unscanned.append((path, f"unreadable: {exc}"))
        return []

    hits = scan_bytes(data, by_len)
    # UTF-16 and other NUL-interleaved encodings do not match a byte window.
    # Re-scan with NULs removed so a UTF-16LE/BE file cannot hide a value.
    if b"\0" in data:
        hits += scan_bytes(data.replace(b"\0", b""), by_len)
    return hits


def main() -> int:
    args = sys.argv[1:]
    if args and args[0] == "--emit-hash":
        # Emit a denylist entry without writing the plaintext anywhere:
        #   .github/scripts/fleet_string_guard.py --emit-hash < value.txt
        #   .github/scripts/fleet_string_guard.py --emit-hash   (paste, Ctrl-D)
        # Avoid `printf '%s' 'value' | ...` — that puts the value in argv,
        # where `ps`, shell history, and agent transcripts all record it.
        value = sys.stdin.buffer.read().strip()
        if not value:
            sys.exit("::error::--emit-hash: read nothing on stdin")
        print(f"{len(value.lower())}:{digest(value)}")
        return 0

    use_allowlist = True
    for arg in args:
        if arg == "--no-allowlist":
            # Reports what the allowlist is currently hiding.
            use_allowlist = False
        else:
            sys.exit(f"::error::unknown argument: {arg}")

    by_len = load_denylist()
    allow = load_allowlist() if use_allowlist else set()

    failed = False
    waived: list[str] = []
    unscanned: list[tuple[str, str]] = []

    for path in tracked_files():
        hits = scan_file(path, by_len, unscanned)
        if not hits:
            continue
        if path in allow:
            waived.append(f"{path} ({len(hits)} occurrence(s))")
            continue
        if not failed:
            print("::error::Fleet-private strings present in the tree:")
            failed = True
        for lineno, prefix in hits:
            print(f"  {path}:{lineno}: <guarded class {prefix}>")

    # A guard that hides what it waived or skipped is the same defect in a new
    # place — these must be visible on every run, not only on failure.
    for path in waived:
        print(f"::warning::allowlisted, still carries a guarded string: {path}")
    for path, reason in unscanned:
        print(f"::warning::NOT SCANNED ({reason}): {path}")

    if failed:
        print("")
        print("These are deployment-specific and must not appear in this public repo.")
        print("No plaintext above on purpose — this log is public. Locate them with:")
        print("  .github/scripts/fleet_string_guard.py")
        return 1

    total = sum(len(v) for v in by_len.values())
    summary = f"Clean: {total} guarded classes, no unallowlisted occurrences in the tree."
    if unscanned:
        summary += f" ({len(unscanned)} file(s) NOT scanned — see warnings.)"
    print(summary)
    if waived:
        print(f"({len(waived)} allowlisted path(s) still carry one — see warnings.)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
