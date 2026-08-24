#!/usr/bin/env python3
"""Tests for the fleet-private string guard.

Every case uses *synthetic* guarded values against a synthetic git repo. A test
suite for a secret guard must not need a real secret to run — otherwise the
tests become the leak the guard exists to prevent.

Run: .github/scripts/test_fleet_string_guard.py
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

GUARD = Path(__file__).resolve().parent / "fleet_string_guard.py"

# Synthetic. Deliberately shaped like the real classes (hostname-ish, mixed
# separators) without being any of them.
FAKE = ["ZZ-QQ9", "FAKE-BOX-A", "NOPE-SRV"]

PASSED, FAILED = 0, 0


def emit_hash(value: str) -> str:
    res = subprocess.run(
        [sys.executable, str(GUARD), "--emit-hash"],
        input=value.encode(),
        capture_output=True,
        check=True,
    )
    return res.stdout.decode().strip()


def run_guard(root: Path, denylist: Path, allowlist: Path | None, *,
              expected: int = len(FAKE), max_bytes: int | None = None,
              extra_args: list[str] | None = None):
    env = dict(os.environ)
    env["FLEET_GUARD_ROOT"] = str(root)
    env["FLEET_GUARD_DENYLIST"] = str(denylist)
    env["FLEET_GUARD_ALLOWLIST"] = str(allowlist) if allowlist else str(root / "_absent")
    env["FLEET_GUARD_EXPECTED_CLASSES"] = str(expected)
    if max_bytes is not None:
        env["FLEET_GUARD_MAX_BYTES"] = str(max_bytes)
    return subprocess.run(
        [sys.executable, str(GUARD), *(extra_args or [])],
        capture_output=True, env=env, cwd=str(root)
    )


def make_repo(tmp: Path, files) -> Path:
    root = tmp / "repo"
    root.mkdir()
    subprocess.run(["git", "init", "-q"], cwd=root, check=True)
    for name, body in files.items():
        p = root / name
        p.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(body, bytes):
            p.write_bytes(body)
        else:
            p.write_text(body)
    subprocess.run(["git", "add", "-A"], cwd=root, check=True)
    return root


def check(name: str, cond: bool, detail: str = "") -> None:
    global PASSED, FAILED
    if cond:
        PASSED += 1
        print(f"  ok   {name}")
    else:
        FAILED += 1
        print(f"  FAIL {name} {detail}")


def case(name: str, files, *, expect_fail: bool, deny=None,
         allow: str | None = None, want_in_out: str | None = None,
         not_in_out: list[str] | None = None, raw_deny: str | None = None,
         expected: int | None = None, max_bytes: int | None = None,
         extra_args: list[str] | None = None) -> None:
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        root = make_repo(tmp, files)
        dl = tmp / "deny.sha256"
        if raw_deny is not None:
            dl.write_text(raw_deny)
        else:
            entries = FAKE if deny is None else deny
            dl.write_text("# synthetic\n" + "\n".join(emit_hash(v) for v in entries) + "\n")
        al = None
        if allow is not None:
            al = tmp / "allow.txt"
            al.write_text(allow + "\n")
        res = run_guard(root, dl, al,
                        expected=len(FAKE) if expected is None else expected,
                        max_bytes=max_bytes, extra_args=extra_args)
        out = (res.stdout + res.stderr).decode()
        got_fail = res.returncode != 0
        check(name, got_fail == expect_fail,
              f"(exit={res.returncode}, expected {'nonzero' if expect_fail else '0'})\n{out}")
        if want_in_out is not None:
            check(f"{name} :: output contains {want_in_out!r}", want_in_out in out, f"\n{out}")
        for bad in not_in_out or []:
            check(f"{name} :: output omits {bad!r}", bad not in out, f"\n{out}")


print("fleet-private string guard")

# --- positive controls: the guard must catch these -------------------------
case("catches an exact guarded token",
     {"a.txt": f"host = {FAKE[0]}\n"}, expect_fail=True)

case("catches a lower-case variant (old guard was case-sensitive)",
     {"a.txt": f"host = {FAKE[0].lower()}\n"}, expect_fail=True)

case("catches a mixed-case variant",
     {"a.txt": f"host = {FAKE[1].title()}\n"}, expect_fail=True)

case("catches a token embedded in a longer FQDN",
     {"a.txt": f"url = https://{FAKE[0].lower()}.internal.example.com/x\n"}, expect_fail=True)

case("catches a token inside SQL quotes",
     {"m.sql": f"WHEN '{FAKE[2].lower()}' THEN 'CC-X'\n"}, expect_fail=True)

case("scans the whole tree, not just changed lines",
     {"deep/nested/old.md": f"legacy note about {FAKE[1]}\n"}, expect_fail=True)

case("catches every guarded class, not just the first",
     {"a.txt": f"{FAKE[2]}\n"}, expect_fail=True)

# --- the CI log is public: output must carry no plaintext at all ----------
case("reports a digest prefix, never the value or any character of it",
     {"a.txt": f"host = {FAKE[0]}\n"}, expect_fail=True,
     want_in_out="<guarded class ",
     not_in_out=[FAKE[0], FAKE[0].lower(), FAKE[0][0] + "*", "*" + FAKE[0][-1]])

# --- negative controls: the guard must NOT fire ----------------------------
case("clean tree passes", {"a.txt": "nothing to see\n"}, expect_fail=False)

case("similar-but-different string does not fire",
     {"a.txt": "host = ZZ-QQ8\nother = FAKE-BOX-B\n"}, expect_fail=False)

case("guarded substring of an unrelated word does not fire",
     {"a.txt": "the word nope and the word srv separately\n"}, expect_fail=False)

# --- allowlist -------------------------------------------------------------
case("allowlisted path is waived",
     {"frozen.sql": f"WHEN '{FAKE[0].lower()}'\n"}, expect_fail=False,
     allow="frozen.sql",
     want_in_out="::warning::allowlisted, still carries a guarded string")

case("allowlist does not waive other paths",
     {"frozen.sql": f"{FAKE[0]}\n", "other.txt": f"{FAKE[0]}\n"}, expect_fail=True,
     allow="frozen.sql")

case("allowlisting a now-clean path emits no warning",
     {"frozen.sql": "clean now\n"}, expect_fail=False,
     allow="frozen.sql", not_in_out=["::warning::"])

# --- fail-closed -----------------------------------------------------------
case("empty denylist refuses to certify",
     {"a.txt": "clean\n"}, expect_fail=True, deny=[],
     want_in_out="expected")

with tempfile.TemporaryDirectory() as td:
    tmp = Path(td)
    root = make_repo(tmp, {"a.txt": "clean\n"})
    res = run_guard(root, tmp / "does-not-exist.sha256", None)
    check("missing denylist refuses to certify", res.returncode != 0)
    check("missing denylist says so",
          "denylist missing" in (res.stdout + res.stderr).decode())

# --- the guard must not leak via its own artifacts -------------------------

# --- W1: a value glued to a prefix/suffix without a separator --------------
case("catches a value with no separator boundary (prefix and suffix)",
     {"a.txt": f"xx{FAKE[0].lower()}yy\n"}, expect_fail=True)

case("catches a value inside a URL path segment",
     {"a.txt": f"https://example.com/a/{FAKE[2].lower()}/b\n"}, expect_fail=True)

# --- C2: encodings and NUL bytes must not hide a value --------------------
case("catches a value in a file with NUL bytes (was skipped as binary)",
     {"a.bin": b"\x00\x00head\x00 " + FAKE[0].lower().encode() + b" tail\x00"},
     expect_fail=True)

case("catches a value stored UTF-16LE",
     {"a.txt": ("host = " + FAKE[1]).encode("utf-16-le")}, expect_fail=True)

# --- C2: over-cap files are reported, never silently certified ------------
case("a file over the size cap is reported as NOT SCANNED, not skipped quietly",
     {"big.txt": "x" * 4096 + "\n" + FAKE[0] + "\n"}, expect_fail=False,
     max_bytes=1024, want_in_out="::warning::NOT SCANNED")

case("an over-cap file makes the clean summary say so",
     {"big.txt": "x" * 4096 + "\n"}, expect_fail=False,
     max_bytes=1024, want_in_out="NOT scanned")

# --- C1: a malformed or short denylist must never certify -----------------
case("malformed denylist entry aborts instead of silently dropping a class",
     {"a.txt": f"{FAKE[0]}\n"}, expect_fail=True,
     raw_deny="# synthetic\nTraceback most recent call last\n",
     want_in_out="malformed entry")

case("truncated digest aborts",
     {"a.txt": f"{FAKE[0]}\n"}, expect_fail=True,
     raw_deny="# synthetic\n6:0ca3dac0\n", want_in_out="malformed entry")

case("a dropped class is a red build, not a quietly narrower guard",
     {"a.txt": "clean\n"}, expect_fail=True, deny=FAKE[:2],
     want_in_out="expected 3")

# --- W6: argument handling -------------------------------------------------
case("an unknown argument is an error, not a silent full scan",
     {"a.txt": "clean\n"}, expect_fail=True, extra_args=["--emit-hashes"],
     want_in_out="unknown argument")

# --- N6: --no-allowlist makes the waived content countable -----------------
case("--no-allowlist re-fails an allowlisted path",
     {"frozen.sql": f"{FAKE[0].lower()}\n"}, expect_fail=True,
     allow="frozen.sql", extra_args=["--no-allowlist"])

case("the waiver warning reports how much it is hiding",
     {"frozen.sql": f"{FAKE[0]}\n{FAKE[0]}\n"}, expect_fail=False,
     allow="frozen.sql", want_in_out="2 occurrence(s)")

# --- W5: an unreadable file must not count as clean ------------------------
with tempfile.TemporaryDirectory() as td:
    tmp = Path(td)
    root = make_repo(tmp, {"a.txt": "clean\n"})
    dl = tmp / "deny.sha256"
    dl.write_text("\n".join(emit_hash(v) for v in FAKE) + "\n")
    (root / "a.txt").chmod(0o000)
    res = run_guard(root, dl, None)
    out = (res.stdout + res.stderr).decode()
    (root / "a.txt").chmod(0o644)
    if os.geteuid() == 0:
        print("  skip unreadable-file case (running as root)")
    else:
        check("unreadable file is reported, not counted clean",
              "::warning::NOT SCANNED" in out and "unreadable" in out, f"\n{out}")

# --- performance guard: no combinatorial blowup on separator-rich tokens ---
with tempfile.TemporaryDirectory() as td:
    import time
    tmp = Path(td)
    blob = "-".join(f"p{i}" for i in range(4000))  # base64url-ish, 4000 parts
    root = make_repo(tmp, {"vendor.js": blob + "\n"})
    dl = tmp / "deny.sha256"
    dl.write_text("\n".join(emit_hash(v) for v in FAKE) + "\n")
    t0 = time.monotonic()
    res = run_guard(root, dl, None)
    elapsed = time.monotonic() - t0
    check("a 4000-part token scans in linear time (no candidate explosion)",
          res.returncode == 0 and elapsed < 10, f"(took {elapsed:.1f}s, exit={res.returncode})")

print(f"\n{PASSED} passed, {FAILED} failed")
sys.exit(1 if FAILED else 0)
