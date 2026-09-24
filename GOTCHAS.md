# Gotchas

Traps in this repo: the rule, why it bites, and the check. History lives in `git log` and `CHANGELOG.md`. `CLAUDE.md` → *Gotchas* is canonical for what it covers; entries here add to it.

## Database Migrations

- **Inventory, incident, and ticketing tables are gone on purpose.** Do not add them back; scope and reasons are in `ROADMAP.md`. `knowledge.source_incident_id` went with them — provenance is the `author` column.
- **A fresh disposable database needs pgvector installed before migrations run.** The `ops_brain` role is deliberately not a superuser and cannot `CREATE EXTENSION`. Create the database owned by `ops_brain`, then have a privileged role run `CREATE EXTENSION IF NOT EXISTS vector` in it. CI does the same. Do not grant the app role superuser for convenience.
- **Editing an unreleased migration that already ran locally gives `VersionMismatch(<version>)` at boot.** The checksum drifted. `DELETE FROM _sqlx_migrations WHERE version = <N>;`, drop what the migration created, re-run. Only while the migration is unmerged — never after.
- **`migrations/20260426000002_normalize_handoff_machine_names.sql` cannot be edited, not even its comments.** `src/db.rs` runs `sqlx::migrate!` at startup, and sqlx returns `VersionMismatch` when an applied migration's checksum changes; there is no ignore-checksum option (`set_ignore_missing` covers *missing* migrations only). Any edit stops the deployed instance booting until `_sqlx_migrations` is updated in lockstep on the production database, which is why the string guard allowlists the file.
- **Squashing migrations to a baseline is worse than editing one, not an escape from it.** sqlx's `run_direct` applies any resolved migration whose version is absent from `_sqlx_migrations` and has no version-ordering check, so a new baseline *executes against populated production data*. Reusing an applied version gives `VersionMismatch` instead. Decision and exit condition: `TODO.md`. As of sqlx-core 0.8.6; re-read `migrate/migrator.rs` on upgrade.

## Commit workflow

- **The fleet-private string guard is hash-based and scans the whole tracked tree.** `.github/scripts/fleet_string_guard.py` (no arguments, no secret) compares salted SHA-256 digests against `.github/fleet-denylist.sha256`; CI runs the same script from `.github/workflows/secret-scan.yml`. There is no plaintext pattern anywhere; do not reconstruct one by hand. Run it before every push. `--text [FILE…]` (stdin if none) scans free text instead. The tree scan cannot see commit messages or PR bodies, and both have leaked. CI scans the PR title/body/branch and each commit message and identity in the pushed/PR range, never the whole history: merged history still carries pre-scrub messages and would fail every run. `.githooks/commit-msg` runs only on `git commit`: not with `--no-verify`, and not for cherry-pick, rebase picks or `git am`. CI is the only net for those. It needs `python3`, so a Windows Git Bash without it is blocked, not waved through.
  - Matching is case-insensitive and position-independent (a byte window slides over each identifier run, and NUL-stripped bytes are re-scanned for UTF-16), so a value glued to a prefix or a URL segment is caught.
  - A hit prints `path:line: <guarded class 0ca3dac0>` — a digest prefix, never plaintext. Denylist entries are `<length>:<digest>`; the published length is deliberate and is what makes the windowed scan possible.
  - It fails closed on a missing denylist, a malformed entry, an entry count that differs from `EXPECTED_CLASSES` in the script, and an unknown argument. An unreadable or over-16 MB file prints `::warning::NOT SCANNED`.
  - Waivers are paths in `.github/fleet-denylist-allow.txt`, each with a reason and an exit condition; a waived path that still carries a guarded string warns on every run. `--no-allowlist` re-scans waived paths to get a count.
  - The public deploy host is not a guarded class: it is publicly resolvable and in Certificate Transparency logs.
- **Add a guarded class without writing the value down.** `.github/scripts/fleet_string_guard.py --emit-hash < value.txt` (or run it bare, paste, Ctrl-D), append the emitted line to the denylist, bump `EXPECTED_CLASSES`. Never `printf '%s' 'value' | …` — that puts the value in `argv`, where `ps`, shell history, and agent transcripts record it.
- **A `.gitignore` pattern with a trailing slash ignores directories only — a symlink of the same name is not ignored.** A `node_modules` symlink then gets staged by a broad `git add` as a mode-120000 blob whose content is the author's absolute path. `adapters/*/.gitignore` use `node_modules` with no slash; keep it that way. Pre-push check: `git ls-files -s | awk '$1=="120000"'` should print nothing.
- **`git commit -a` omits untracked files the build already depends on.** Every local check is green because the file is on disk, while the commit is one file short. Run `git status --short` and look for `??` before committing a change that adds a required script, migration, or fixture.

## Production deploy checks

- **Port 3000, the prod smoke commands, and `-f docker-compose.prod.yml`:** see `CLAUDE.md` → *Gotchas*.
- **`/health` and `/ready` are unauthenticated on purpose.** `bearer_auth` in `src/auth.rs` skips both so Docker and the reverse proxy can probe without a bearer. `/ready` runs a minimal database check and returns no detail. `/api`, `/mcp`, and `/live` stay protected.
- **The `## Surface (N tools)` heading in `CLAUDE.md` is machine-parsed.** An external deploy procedure extracts `N` with `sed -n 's/^## Surface (\([0-9]\+\) tools).*/\1/p' CLAUDE.md` and compares it with the live MCP `tools/list`. Nothing in this repo parses it, so it looks decorative. Keep the heading format exact and keep `N` equal to the number of `#[tool(` stubs in `src/tools/mod.rs`.
- **After any prod compose action, confirm the container is on the real database:** `docker exec shared-postgres psql -U ops_brain -d ops_brain -c "SELECT count(*) FROM handoffs;"`. Zero means the wrong database.

## Compose files

- **Two compose files, two databases.**

  | File | Purpose | Database |
  |---|---|---|
  | `docker-compose.yml` | dev / new user; project name `ops-brain-dev` | bundled postgres, fresh `pgdata` |
  | `docker-compose.prod.yml` | prod; joins external `web-net` + `shared-db` networks | `shared-postgres` |

  Without `-f docker-compose.prod.yml` on the prod host you get an isolated, empty `ops-brain-dev` stack; production is untouched. Remove the orphans: `docker stop` and `docker rm` `ops-brain-dev ops-brain-dev-db`, then `docker network rm ops-brain-dev_default` and `docker volume rm ops-brain-dev_pgdata`.
- **A new env var needs both `.env` and `docker-compose.prod.yml`:** see `CLAUDE.md` → *Gotchas*. The symptom is a startup log showing the default, e.g. `HTTP allowed_hosts: loopback default`. All config is read through clap `#[arg(env = …)]` in `src/config.rs` (the binary never calls `std::env::var`), so that file is the complete list to diff against the compose `environment:` block.
- **The both-places rule covers only variables the binary reads.** `scripts/operator-notify.sh` runs on the host under cron, so its `OPS_NOTIFY_*` and `OPS_BRAIN_URL` settings are set by the crontab entry (`docs/operator-notify.md`). Putting them in the compose `.env` or the compose file looks configured while the cron job runs without them. Only the poller's credential follows the rule: it rides in `OPS_BRAIN_MACHINE_TOKENS`, which the server reads.

## MCP Clients

- **`Session not found` after idle is server-side eviction.** rmcp's default session `keep_alive` is 300 s; `src/main.rs` raises it to 3600 s. MCP clients do not reliably re-initialize on the resulting 404 — reconnect (`/mcp` in Claude Code). If it recurs sooner, check that the deployed binary carries the bump, then rmcp's release notes.
- **Recreating the container evicts every MCP session, including the deployer's.** The session table is in memory. `complete_handoff` then returns `Session not found`, and retrying in the same turn does not reconnect. Close out over SQL instead:
  `docker exec shared-postgres psql -U ops_brain -d ops_brain -c "UPDATE handoffs SET status='completed', commit_hash='<sha>', updated_at=NOW() WHERE id='<uuid>'"`.
  The keep-alive bump does not affect this.
- **PowerShell's legacy native-argument passing (7.2) strips embedded quotes from inline JSON.** The Windows Claude launcher therefore hands its server definition to the Node overlay helper as one native argument. Do not move it back to an inline `--mcp-config` argument, which is also invisible to the Channel resolver (see *Live messaging*).
- **The Windows launchers require PowerShell 7.4 / .NET 8.** Older runtimes ignore `Start-Process -WindowStyle Hidden` when output is redirected and leave blank consoles open. Do not switch to `-NoNewWindow`: the helper then shares the launcher's console input and freezes TUI keystrokes. Pass Codex App Server's `--listen` value as bare `ws://IP:PORT`; `[uri].AbsoluteUri` appends a slash that Codex rejects. See the comments in `scripts/ops-brain-codex-live.ps1`.

## Machine-Filed Handoffs

- **Replies to a machine-filed handoff never reach the agent that operates the producer.** `from_agent` on an `origin=machine` handoff is the token binding (`docs/machine-callers.md`), and `list_replies_to_me` matches `parent.from_agent`. When replying to one, also notify the operating agent directly, or address the reply to it.

## Auth & identity (per-agent tokens)

Contract: `docs/agent-tokens.md`.

- **Identity enforcement depends on a single `http` crate version shared by `axum` and `rmcp`.** `bearer_auth` puts `CallerClass` into the request extensions, rmcp hands that request's `http::request::Parts` to the tool context, and `helpers::bound_agent` downcasts it by `TypeId`. With two `http` versions the downcast misses silently and every caller looks unbound. After a dependency change `cargo tree -i http` must show one version; the identity-mismatch tests in `tests/integration.rs` cover the whole path.
- **Renaming an agent is a data cutover, not a code change.** Slugs are free-form, so no deploy renames anyone. Rebind `from_agent` in `OPS_BRAIN_AGENT_TOKENS` (keep the token value), add the new name next to the old one in each `OPS_BRAIN_MACHINE_TOKENS` `agents` list until producers flip, and rewrite `handoffs.from_agent`/`to_agent` + `knowledge.author`. Tell each renamed agent **before** the rewrite: afterwards a session still using the old slug gets an empty `check_in` with no error (only writes fail loud). Run of record: the 2026-09-24 `CC-*` → `Claude-*` rename (#135).
- **A bulk `UPDATE` on `handoffs` or `knowledge` re-ages the whole history.** `trg_handoffs_updated_at` / `trg_knowledge_updated_at` stamp `updated_at = now()` on every touched row, which resets briefing ages and stuck detection. Wrap data fixes in `ALTER TABLE … DISABLE TRIGGER …` / `ENABLE TRIGGER …` inside one transaction, and check that max `updated_at` is unchanged afterwards.
- **rmcp's `Extension<T>` extractor errors when `T` is absent; take the `Extensions` bag instead.** `T` is absent on every stdio call and under `--dev-no-auth`. Handlers that read caller identity take `ext: rmcp::model::Extensions` and treat absence as unbound. On the HTTP listener `bearer_auth` always inserts a `CallerClass` first.

## Handoff threads and IDs

- **A truncated handoff ID is ambiguous — thread on the full UUID.** IDs are UUIDv7, so handoffs filed seconds apart share their leading hex. A reply on the wrong thread fails quietly: `complete_handoff` on someone else's closed handoff returns `Handoff is already completed`, which reads like success. Keep full UUIDs in notes and commits, and after replying confirm the parent's `to_agent` is you.
- **Completing a parent does not complete its replies.** An `action` reply is independent open work. When closing a thread, complete each action reply explicitly, then run `check_in` once more to catch an orphan. `notify` replies need no cleanup.

## GitHub PR Workflow

- **Every agent commits as the same GitHub account, so none can approve another's PR.** `gh pr review --approve` fails with `Can not approve your own pull request`. Post the review with `gh pr comment`. Do not add a required-reviewer rule to this repo; it would block every merge.
- **A PR that conflicts with its base runs no checks at all.** `pull_request` workflows build `refs/pull/N/merge`, which does not exist while the branch conflicts, so nothing runs — the string guard included — and the page looks merely queued. Rebase first, then read the checks.
- **A stacked PR runs no checks either.** `ci.yml` and `secret-scan.yml` trigger on `pull_request: branches: [main]`, so a PR based on another branch matches nothing and still shows `mergeStateStatus: CLEAN`. Never merge it on the strength of the base's checks. Wait for the retarget to `main` to produce its own run, and meanwhile run the guard and the relevant suites by hand.
- **`gh pr merge --delete-branch` on a stacked PR's base closes the stacked PR.** A closed PR whose base ref is gone can be neither reopened nor retargeted. Recover with `git rebase --onto main <old-base-tip> <branch>`, a force-push, and a new PR. When merging a stack bottom-up, leave `--delete-branch` off until the top is in.
- **Editing a PR body does not retract it.** GitHub keeps every prior revision, readable by anyone on a public repo; the same holds for issue and comment bodies:

  ```
  gh api graphql -f query='{ repository(owner:"OWNER",name:"REPO") {
    pullRequest(number:N) { userContentEdits(first:20){ totalCount nodes{ editedAt deletedAt diff } } } } }'
  ```

  Treat a string that was ever in a PR body as published. A force-push does not unpublish the old commit either, and its blobs are served separately by the blobs API, so a cleanup request must name both. Verify reachability against a control: a fabricated SHA returns 422 for commits and 404 for blobs.
- **After a squash merge, "not on any remote branch" says nothing about whether the content shipped.** Squashing rewrites SHAs, so `git branch -r --contains` and `git cherry` report shipped work as unpushed, and genuinely unpushed commits look the same. Before resetting a diverged branch, compare content: `git diff --stat <local-branch> <squash-commit>` — an empty diff means it shipped. Tag first (`git tag rescue/<branch>-<date>`).
- **Local `main` drifts by default, because this repo squash-merges.** `--delete-branch` drops you onto a local `main` that still holds pre-squash commits. Fast-forward or reset it after every merge. Launcher symlinks resolve through the checkout (see *Live messaging*), so a stale `main` silently runs the old client.
- **Cancelling a duplicate Actions run can turn a green PR red.** Close-and-reopen starts a second check suite for the same commit, the PR rollup follows the newest run of each check, and a cancelled run counts as not green. Let the newest required checks finish.

## Live messaging (Channels / adapters)

Canonical: `docs/live-messaging.md` (protocol, receipts) and `docs/live-fleet-rollout.md` ("rollout doc": install, launchers, logs, acceptance gate).

- **A registered live peer is not a receiving peer.** The adapter registers on `/live` independently of whether the client session ever bound, so `list_live_peers` can look healthy and `send_live_message` can return `host_accepted` while the session receives nothing. Prove a lane with a marker read back from the other side. Rollout doc → *Per-host acceptance gate*.
- **A source-checkout install is symlinks into the checkout, not a snapshot.** A branch switch or pull changes what the installed commands run. Rollout doc → *Linux installation and launch*.
- **The Channel resolver ignores `--mcp-config` servers, and `CLAUDE_CONFIG_DIR` replaces the user config rather than layering over it.** The launcher's private per-launch overlay exists because of both. Do not replace it with ambient user or local registration. Rollout doc → *Claude Channel resolver isolation*.
- **Claude Code discards the adapter's stderr.** `Successfully connected (transport: stdio)` in Claude's MCP log is the stdio transport, not `/live`. The only evidence of a bound lane is `claude-adapter.*.log` under `OPS_BRAIN_LIVE_STATE_DIR`. On Windows that log may end at `live adapter connected` after a clean exit. Rollout doc → *Reading the adapter logs*.
- **PowerShell profile functions forward every argument to the client**, including ones that look like launcher options. Use the `-live` compatibility commands or the `.ps1` scripts for `-Mode` / `-ProfileFile`, quote a literal `'--'`, and open a new terminal after changing the profile. Rollout doc → *Explicit launchers (Windows)*.
- **`thread/resume failed: no rollout found` at Codex adapter startup is the selection mechanism, not a fault.** `thread/loaded/list` is process-global and includes an in-memory thread with no rollout on a clean launch, so discovery keys on `thread/resume` succeeding and treats *every* rejection as "not a candidate". On a shared or multi-thread App Server pin `OPS_BRAIN_CODEX_THREAD_ID` (`adapters/codex-app-server/README.md`). The decisive warning goes to `codex-adapter.*.log`, not to a `script -q -c` receipt.
- **In `#resolveThread()` (`adapters/codex-app-server/src/bridge.mjs`), latch a discovered thread id only after `thread/resume` succeeds, and clear it on failure.** Latching first strands the adapter on a dead id (a brand-new TUI thread has no rollout yet). Never clear a *configured* id: falling back to "the one loaded thread" can retarget another agent's session. Keep `cause` and `delivery_stage` on the warnings, or transport loss and an unresumable thread log identically.
- **`thread/loaded/list` needs the `initialize` + `initialized` handshake first.** A bare connection returns `{"code":-32600,"message":"Not initialized"}`.
- **`claude <anything> --version` exits 0, so it cannot probe whether a flag exists.** `--version` short-circuits argument validation, and `--dangerously-load-development-channels` is absent from `--help`. Grep the installed binary for the flag name, with a known flag as a positive control.
- **`claude -p` does not evaluate development channels.** A bogus `server:` name produces no warning in `-p` mode, so a headless "channel resolved" result is void. Test channel binding interactively, alongside a bogus-name control.
- **A TTY probe inside `$(...)` is always false.** In `scripts/ops-brain-claude-live`, `auto_passthrough_reason` must be called for its exit status, never inside a command substitution, or `--auto` passes through on a real terminal. The pty-driven case in `scripts/test-live-launchers` is the one that covers it.
- **In adapter tests, wait on a parsed frame, never on a substring of raw stdout.** The channel instructions in the initialize response already contain `lane_status`, so a regex wait can return before the notification exists. Use `waitForFrame` in `adapters/claude-channel/test/main.test.js`, which also tolerates a half-written trailing line. A failure that arrives far faster than its timeout is not a timeout — do not widen the deadline.
- **A sandbox that denies loopback `bind()` errors every socket-backed adapter test.** This happens in headless or sandboxed agent sessions. It means blocked, not failing: re-run `npm test` in an ordinary interactive shell before reporting the suite as broken.

## Client bundle / dependency manifest

- **A CHANGELOG version does not prove a release artifact exists.** Some versions have changelog sections and no tag. Check `git tag` or GitHub Releases before telling anyone to install a named bundle.
- **Test bundle-shape invariants against a real bundle.** The fixture in `scripts/test-live-launchers` is hand-built and has no `DEPENDENCIES.json`, which every shipped bundle has. Per-client dependency scoping and the manifest-deleted case belong in `scripts/test-client-bundle`, which builds the bundle and perturbs it.
- **Run adapter tests as bare `node --test` from the adapter directory** (what `npm test` does). `node --test test/` fails with `MODULE_NOT_FOUND` and reports `fail 1`, which looks like a real failure. Seen on Node 26; re-check on upgrade.
- **`scripts/test-live-launchers` needs the adapter dependencies installed.** A fresh worktree has no `adapters/*/node_modules`, and the suite stops with `adapter dependencies are not installed. This is not a launcher failure.` Run `npm --prefix adapters/claude-channel ci --ignore-scripts` and the same for `adapters/codex-app-server`. CI installs them first, so this is local-only.
- **That suite must never read your real profile.** Every launch in it pins `XDG_CONFIG_HOME` to an unconfigured directory (or goes through `LIVE_ENV`); an assertion added outside that passes on a configured host and fails in CI. Control before pushing: `XDG_CONFIG_HOME=$(mktemp -d) scripts/test-live-launchers` must agree with a normal run.
- **A UTF-8 BOM on a `.ps1` fails the Linux CI job with a misleading message.** `scripts/test-live-launchers` anchors `^#requires -Version 7\.4$` on line 1 and the BOM sits in front of the `#`, so it reads as a missing `#requires` line. Windows editors add the BOM silently. Check with `head -c3 <file> | xxd -p` (`efbbbf`); write with `[Text.UTF8Encoding]::new($false)`.
- **`sed -i` on a CRLF file under Git Bash rewrites it to LF, and MSYS tools hide it.** Afterwards `grep -c $'\r$'` still reports CRLF, and with `core.autocrlf=true` `git diff --stat` is clean. Only a byte count taken outside MSYS is trustworthy (`[IO.File]::ReadAllBytes($p).Length`). Edit CRLF files from PowerShell.
