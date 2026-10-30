#!/usr/bin/env python3
"""Synthetic event and publishing tests. No network or real guarded values."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
GUARD = ROOT / ".github/scripts/fleet_string_guard.py"
WRITER = ROOT / "scripts/guarded-github-write.py"
FAKE = "ZZ-QQ9"
EVENTS = {
    "pull_request_target": {"pull_request": {
        "title": "Fix a bug", "body": "Checked locally", "head": {"ref": "fix/bug"},
        "base": {"ref": "main"}, "mergeable": False}},
    "issues": {"issue": {"title": "A bug", "body": "Steps to reproduce"}},
    "issue_comment": {"comment": {"body": "A reply"}},
    "pull_request_review": {"review": {"body": "Reviewed"}},
    "pull_request_review_comment": {"comment": {"body": "A finding", "path": "src/main.rs"}},
    "release": {"release": {"name": "A release", "body": "Changes", "tag_name": "v1.0.0",
                              "target_commitish": "main"}},
}


class Fixtures(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        denylist = self.directory / "deny.sha256"
        digest = hashlib.sha256(b"ops-brain-fleet-guard-v1:" + FAKE.lower().encode()).hexdigest()
        denylist.write_text(f"{len(FAKE)}:{digest}\n")
        self.environment = dict(os.environ, FLEET_GUARD_DENYLIST=str(denylist),
                                FLEET_GUARD_EXPECTED_CLASSES="1")
        self.marker = self.directory / "request.json"
        fake_gh = self.directory / "gh"
        fake_gh.write_text(f"#!{sys.executable}\n" + """
import json
import os
from pathlib import Path
import sys

if os.environ.get('REWRITE_INPUT'):
    Path(os.environ['REWRITE_INPUT']).write_text('changed after scanning')
Path(os.environ['REQUEST_MARKER']).write_text(json.dumps({
    'args': sys.argv[1:], 'body': sys.stdin.read()}))
if os.environ.get('FAIL_GH'):
    print(os.environ['FAIL_GH'], file=sys.stderr)
    sys.exit(7)
""")
        fake_gh.chmod(0o755)
        self.environment.update(PATH=str(self.directory) + os.pathsep + os.environ["PATH"],
                                REQUEST_MARKER=str(self.marker))

    def run_event(self, name, payload, *, raw=None):
        path = self.directory / "event.json"
        path.write_bytes(raw if raw is not None else json.dumps(payload).encode())
        return subprocess.run([sys.executable, str(GUARD), "--github-event", str(path)],
                              env=dict(self.environment, GITHUB_EVENT_NAME=name), capture_output=True)

    def run_write(self, payload, *, method="POST", endpoint="repos/owner/repo/pulls",
                  raw=None, dry_run=False, via_stdin=False):
        path = self.directory / "input.json"
        data = raw if raw is not None else json.dumps(payload).encode()
        path.write_bytes(data)
        command = [sys.executable, str(WRITER), method, endpoint, "--input",
                   "-" if via_stdin else str(path)]
        if dry_run:
            command.append("--dry-run")
        return subprocess.run(command, input=data if via_stdin else None,
                              env=self.environment, capture_output=True)

    def assert_blocked(self, result):
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.marker.exists(), "blocked request reached gh")
        output = (result.stdout + result.stderr).decode().lower()
        self.assertNotIn(FAKE.lower(), output)


class EventTests(Fixtures):
    def test_each_supported_event_passes_clean_text_and_rejects_guarded_body(self):
        for name, event in EVENTS.items():
            with self.subTest(name=name):
                self.assertEqual(self.run_event(name, event).returncode, 0)
                dirty = json.loads(json.dumps(event))
                next(iter(dirty.values()))["body"] = "Details: " + FAKE.lower()
                self.assert_blocked(self.run_event(name, dirty))

    def test_pr_titles_and_branches_are_scanned_even_with_conflicts(self):
        for field in ["title", "head", "base"]:
            with self.subTest(field=field):
                event = json.loads(json.dumps(EVENTS["pull_request_target"]))
                if field == "title":
                    event["pull_request"][field] = FAKE
                else:
                    event["pull_request"][field]["ref"] = "fix/" + FAKE
                self.assert_blocked(self.run_event("pull_request_target", event))

    def test_release_names_tags_targets_and_comment_paths_are_scanned(self):
        for field in ["name", "tag_name", "target_commitish"]:
            with self.subTest(field=field):
                event = json.loads(json.dumps(EVENTS["release"]))
                event["release"][field] = FAKE
                self.assert_blocked(self.run_event("release", event))
        self.assert_blocked(self.run_event("pull_request_review_comment",
                                           {"comment": {"body": "clean", "path": FAKE + ".rs"}}))

    def test_nullable_bodies_and_release_names(self):
        for name, event in EVENTS.items():
            with self.subTest(name=name):
                nullable = json.loads(json.dumps(event))
                next(iter(nullable.values()))["body"] = None
                if name == "release":
                    nullable["release"]["name"] = None
                self.assertEqual(self.run_event(name, nullable).returncode, 0)

    def test_json_escapes_and_nuls_do_not_hide_text(self):
        self.assert_blocked(self.run_event("issue_comment", {},
                                           raw=b'{"comment":{"body":"Z\\u005a-QQ9"}}'))
        self.assert_blocked(self.run_event("issue_comment", {"comment": {"body": "ZZ-\0QQ9"}}))

    def test_unselected_envelope_is_not_scanned_or_echoed(self):
        event = {"comment": {"body": "clean"}, "sender": {"login": FAKE}, "ignored": FAKE}
        result = self.run_event("issue_comment", event)
        self.assertEqual(result.returncode, 0)
        self.assertNotIn(FAKE.encode(), result.stdout + result.stderr)

    def test_invalid_or_unsupported_event_fails_without_payload_in_output(self):
        cases = [("unknown", EVENTS["issues"]), ("issues", {}), ("issues", []),
                 ("issues", {"issue": {"title": [FAKE], "body": "clean"}}),
                 ("issues", {"issue": {"title": None, "body": "clean"}})]
        for name, event in cases:
            with self.subTest(name=name, event=event):
                self.assert_blocked(self.run_event(name, event))
        self.assert_blocked(self.run_event("issues", {}, raw=("invalid " + FAKE).encode()))


class PublishingTests(Fixtures):
    def test_clean_payload_is_sent_as_data_and_only_once(self):
        marker = self.directory / "shell-must-not-run"
        payload = {"title": "Fix", "body": f"Literal $(touch {marker}) and `false`\nNext line",
                   "head": "fix/bug", "base": "main"}
        self.environment["REWRITE_INPUT"] = str(self.directory / "input.json")
        result = self.run_write(payload)
        self.assertEqual(result.returncode, 0, result.stderr)
        recorded = json.loads(self.marker.read_text())
        self.assertEqual(json.loads(recorded["body"]), payload)
        self.assertEqual(recorded["args"], ["api", "--method", "POST", "repos/owner/repo/pulls",
                                            "--input", "-", "--silent"])
        self.assertFalse(marker.exists())
        self.assertNotIn(payload["body"].encode(), result.stdout + result.stderr)

    def test_body_title_branch_keys_and_nested_comments_are_guarded(self):
        dirty = [{"body": FAKE}, {"body": "clean", "title": FAKE},
                 {"body": "clean", "head": "fix/" + FAKE}, {"body": "clean", FAKE: "value"},
                 {"body": "clean", "comments": [{"path": "src/main.rs", "body": FAKE}]},
                 {"body": "ZZ-\0QQ9"}]
        for payload in dirty:
            with self.subTest(payload=payload):
                self.assert_blocked(self.run_write(payload))
        self.assert_blocked(self.run_write({}, raw=b'{"body":"Z\\u005a-QQ9"}'))

    def test_endpoint_is_scanned_and_unsupported_routes_never_reach_gh(self):
        self.assert_blocked(self.run_write({"body": "clean"}, endpoint=f"repos/{FAKE}/repo/issues"))
        for endpoint in ["graphql", "https://api.example.com/repos/a/b/issues", "repos/a/b/issues?x=1",
                         "repos/a/b/contents/file", "repos/a/../issues"]:
            with self.subTest(endpoint=endpoint):
                self.assert_blocked(self.run_write({"body": "clean"}, endpoint=endpoint))
        self.assert_blocked(self.run_write({"body": "clean"}, method="DELETE"))

    def test_supported_public_text_routes(self):
        routes = {
            "POST": ["pulls", "issues", "issues/1/comments", "pulls/1/reviews",
                     "pulls/1/comments", "pulls/1/comments/2/replies",
                     "releases"],
            "PATCH": ["pulls/1", "issues/1", "issues/comments/2", "pulls/comments/2", "releases/1"],
            "PUT": ["pulls/1/reviews/2"],
        }
        for method, suffixes in routes.items():
            for suffix in suffixes:
                with self.subTest(method=method, suffix=suffix):
                    result = self.run_write({"body": "clean"}, method=method,
                                            endpoint="repos/a/b/" + suffix, dry_run=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.marker.exists())

    def test_stdin_and_dry_run_do_not_need_a_live_github_connection(self):
        result = self.run_write({"body": "clean"}, via_stdin=True, dry_run=True)
        self.assertEqual(result.returncode, 0)
        self.assertFalse(self.marker.exists())

    def test_unscanned_defaults_and_invalid_json_are_refused(self):
        for payload in [{}, [], {"body": None}, {"body": 1},
                        {"body": "clean", "generate_release_notes": True}]:
            with self.subTest(payload=payload):
                self.assert_blocked(self.run_write(payload))
        self.assert_blocked(self.run_write({}, raw=("invalid " + FAKE).encode()))

    def test_stored_drafts_cannot_be_published_without_scanning_their_text(self):
        self.assert_blocked(self.run_write({"body": "clean", "draft": False}, method="PATCH",
                                           endpoint="repos/a/b/releases/1"))
        self.assert_blocked(self.run_write({"body": "clean", "event": "COMMENT"},
                                           endpoint="repos/a/b/pulls/1/reviews/2/events"))

    def test_missing_input_or_denylist_never_reaches_gh(self):
        self.environment["FLEET_GUARD_DENYLIST"] = str(self.directory / "missing")
        self.assert_blocked(self.run_write({"body": "clean"}))
        result = subprocess.run([sys.executable, str(WRITER), "POST", "repos/a/b/issues", "--input",
                                 str(self.directory / "missing")],
                                env=self.environment, capture_output=True)
        self.assert_blocked(result)

    def test_argument_errors_do_not_echo_unrecognized_values(self):
        result = subprocess.run([sys.executable, str(WRITER), "POST", "repos/a/b/issues",
                                 "--input", "-", "--body", FAKE],
                                input=b'{"body":"clean"}', env=self.environment, capture_output=True)
        self.assert_blocked(result)

    def test_upstream_errors_are_not_echoed(self):
        self.environment["FAIL_GH"] = FAKE
        result = self.run_write({"body": "clean"})
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(self.marker.exists())
        self.assertNotIn(FAKE.encode(), result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
