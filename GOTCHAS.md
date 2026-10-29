# Gotchas

Traps in this repo: the rule, why it bites, and the check. History lives in `git log` and `CHANGELOG.md`. `CLAUDE.md` → *Gotchas* is canonical for what it covers; entries here add to it.

## Database Migrations

- **Inventory, incident, and ticketing tables are gone on purpose.** Do not add them back; scope and reasons are in `ROADMAP.md`. `knowledge.source_incident_id` went with them — provenance is the `author` column.
- **A fresh disposable database needs pgvector installed before migrations run.** The `ops_brain` role is deliberately not a superuser and cannot `CREATE EXTENSION`. Create the database owned by `ops_brain`, then have a privileged role run `CREATE EXTENSION IF NOT EXISTS vector` in it. CI does the same. Do not grant the app role superuser for convenience.
- **Editing an unreleased migration that already ran locally gives `VersionMismatch(<version>)` at boot.** The checksum drifted. `DELETE FROM _sqlx_migrations WHERE version = <N>;`, drop what the migration created, re-run. Only while the migration is unmerged — never after.
- **Every DB test fails with `VersionMissing(<version>)` after switching branches.** The local `ops_brain_test` database is shared across branches, so once a branch's migration has run there, a branch without that file can't migrate it. That's not a code failure. Test such a branch stacked on the one carrying the migration (a throwaway branch plus `git cherry-pick`), which is also the state that deploys. A fresh database needs a privileged role, because `ops_brain` cannot `CREATE DATABASE` either.
- **`migrations/20260426000002_normalize_handoff_machine_names.sql` cannot be edited, not even its comments.** `src/db.rs` runs `sqlx::migrate!` at startup, and sqlx returns `VersionMismatch` when an applied migration's checksum changes; there is no ignore-checksum option (`set_ignore_missing` covers *missing* migrations only). Any edit stops the deployed instance booting until `_sqlx_migrations` is updated in lockstep on the production database, which is why the string guard allowlists the file.
- **Squashing migrations to a baseline is worse than editing one, not an escape from it.** sqlx's `run_direct` applies any resolved migration whose version is absent from `_sqlx_migrations` and has no version-ordering check, so a new baseline *executes against populated production data*. Reusing an applied version gives `VersionMismatch` instead. Decision and exit condition: `TODO.md`. As of sqlx-core 0.8.6; re-read `migrate/migrator.rs` on upgrade.

## Commit workflow

- **The fleet-private string guard is hash-based and scans the whole tracked tree.** `.github/scripts/fleet_string_guard.py` (no arguments, no secret) compares salted SHA-256 digests against `.github/fleet-denylist.sha256`; CI runs the same script from `.github/workflows/secret-scan.yml`. There is no plaintext pattern anywhere; do not reconstruct one by hand. Run it before every push. `--text [FILE…]` (stdin if none) scans free text instead. The tree scan cannot see commit messages or PR bodies. CI scans the PR title/body/branch and each commit message and identity in the pushed/PR range, never the whole history: merged history still carries pre-scrub messages and would fail every run. `.githooks/commit-msg` runs only on `git commit`: not with `--no-verify`, and not for cherry-pick, rebase picks or `git am`. CI is the only net for those. It needs `python3`, so a Windows Git Bash without it is blocked, not waved through.
  - Matching is case-insensitive and position-independent (a byte window slides over each identifier run, and NUL-stripped bytes are re-scanned for UTF-16), so a value glued to a prefix or a URL segment is caught.
  - A hit prints `path:line: <guarded class 0ca3dac0>` — a digest prefix, never plaintext. Denylist entries are `<length>:<digest>`; the published length is deliberate and is what makes the windowed scan possible.
  - It fails closed on a missing denylist, a malformed entry, an entry count that differs from `EXPECTED_CLASSES` in the script, and an unknown argument. An unreadable or over-16 MB file prints `::warning::NOT SCANNED`.
  - Waivers are paths in `.github/fleet-denylist-allow.txt`, each with a reason and an exit condition; a waived path that still carries a guarded string warns on every run. `--no-allowlist` re-scans waived paths to get a count.
  - The public deploy host is not a guarded class: it is publicly resolvable and in Certificate Transparency logs.
- **Add a guarded class without writing the value down.** `.github/scripts/fleet_string_guard.py --emit-hash < value.txt` (or run it bare, paste, Ctrl-D), append the emitted line to the denylist, bump `EXPECTED_CLASSES`. Never `printf '%s' 'value' | …` — that puts the value in `argv`, where `ps`, shell history, and agent transcripts record it.
- **A `.gitignore` pattern with a trailing slash ignores directories only — a symlink of the same name is not ignored.** A `node_modules` symlink then gets staged by a broad `git add` as a mode-120000 blob whose content is the author's absolute path. Write `node_modules` with no slash in any `.gitignore`. Pre-push check: `git ls-files -s | awk '$1=="120000"'` should print nothing.
- **`git commit -a` omits untracked files the build already depends on.** Every local check is green because the file is on disk, while the commit is one file short. Run `git status --short` and look for `??` before committing a change that adds a required script, migration, or fixture.

## Production deploy checks

- **Pushed, merged and deployed are separate states.** The originating agent owns PR/green CI/merge, then Cloud deploys the exact merged SHA and reports verification. Full ownership and fallback rules: `CLAUDE.md` → *Development Workflow*. A branch-head SHA or a pending-PR handoff is not a deployment-ready release.
- **Port 3000, the prod smoke commands, and `-f docker-compose.prod.yml`:** see `CLAUDE.md` → *Gotchas*.
- **`/health` and `/ready` are unauthenticated on purpose.** `bearer_auth` in `src/auth.rs` skips both so Docker and the reverse proxy can probe without a bearer. `/ready` runs a minimal database check and returns no detail. `/api` and `/mcp` stay protected.
- **The `## Surface (N tools)` heading in `CLAUDE.md` is machine-parsed.** An external deploy procedure extracts `N` with `sed -n 's/^## Surface (\([0-9]\+\) tools).*/\1/p' CLAUDE.md` and compares it with the live MCP `tools/list`. Nothing in this repo parses it, so it looks decorative. Keep the heading format exact and keep `N` equal to the number of `#[tool(` stubs in `src/tools/mod.rs`.
- **After any prod compose action, confirm the container is on the real database:** `docker exec shared-postgres psql -U ops_brain -d ops_brain -c "SELECT count(*) FROM handoffs;"`. Zero means the wrong database.

## Compose files

- **Choose the Compose file explicitly; each bundled stack has its own database.**

  | File | Purpose | Database |
  |---|---|---|
  | `docker-compose.yml` | dev / new user; project name `ops-brain-dev` | bundled postgres, fresh `pgdata` |
  | `docker-compose.prod.yml` | prod; joins external `web-net` + `shared-db` networks | `shared-postgres` |
  | `docker-compose.example.yml` | standalone pinned image; loopback HTTP by default | bundled postgres, its own `pgdata` |

  Without `-f docker-compose.prod.yml` on the prod host you get an isolated, empty `ops-brain-dev` stack; production is untouched. Remove the orphans: `docker stop` and `docker rm` `ops-brain-dev ops-brain-dev-db`, then `docker network rm ops-brain-dev_default` and `docker volume rm ops-brain-dev_pgdata`.
- **A new env var needs both `.env` and `docker-compose.prod.yml`:** see `CLAUDE.md` → *Gotchas*. The symptom is a startup log showing the default, e.g. `HTTP allowed_hosts: loopback default`. All config is read through clap `#[arg(env = …)]` in `src/config.rs` (the binary never calls `std::env::var`), so a unit test diffs that list against the compose `environment:` block.
- **An empty `OPS_BRAIN_INSTRUCTIONS_EXTRA` boots cleanly and drops the deployment line.** All three compose files pass `${OPS_BRAIN_INSTRUCTIONS_EXTRA:-}`, so unset or empty serves the built-in instructions only. There is no error. The log `MCP instructions include a deployment addendum` appears only when a line was appended. On this deployment the line is in the `.env` beside `docker-compose.prod.yml`. Compose interpolates that file; an exported shell variable overrides it. Do not put one operator's rule back into the built-in string or a compose default. A line past the remaining initialize budget aborts startup; the cap is `INSTRUCTIONS_EXTRA_MAX` in `src/tools/mod.rs` (213 bytes with the current text).
- **The both-places rule covers only variables the binary reads.** `scripts/operator-notify.sh` runs on the host under cron, so its `OPS_NOTIFY_*` and `OPS_BRAIN_URL` settings are set by the crontab entry (`docs/operator-notify.md`). Putting them in the compose `.env` or the compose file looks configured while the cron job runs without them. Only the poller's credential follows the rule: it rides in `OPS_BRAIN_MACHINE_TOKENS`, which the server reads.

## MCP Clients

- **A plugin can install successfully while exposing no MCP server.** Codex 0.165.0 does this with the portable root `plugin.json` layout in our isolated check. Use `scripts/package-plugin.py --target codex` for the working local compatibility layout; `--target openai --auth oauth` is the separate portable submission artifact. Verify native tool discovery and a handoff, not just catalog visibility. Public PKCE client IDs also need the host's native setting (`oauth.client_id` for Codex, `oauth.clientId` for Claude); the builder emits it. Re-test on client upgrades before changing layouts. Fixture commands and verified scope: `docs/oauth.md`.
- **`Session not found` after idle is server-side eviction.** rmcp's default session `keep_alive` is 300 s; `src/main.rs` raises it to 3600 s. MCP clients do not reliably re-initialize on the resulting 404 — reconnect (`/mcp` in Claude Code). If it recurs sooner, check that the deployed binary carries the bump, then rmcp's release notes.
- **Recreating the container evicts every MCP session, including the deployer's.** The session table is in memory. `complete_handoff` then returns `Session not found`, and retrying with the same session ID does not reconnect. Reconnect or restart the MCP client to initialize a fresh session, then close out through the tool. A stale transport is not a reason to bypass application validation, caller visibility and state-transition safeguards with a direct SQL update. The keep-alive bump does not preserve sessions across a restart.

## Machine-Filed Handoffs

- **Replies to a machine-filed handoff never reach the agent that operates the producer.** `from_agent` on an `origin=machine` handoff is the token binding (`docs/machine-callers.md`), and `list_replies_to_me` matches `parent.from_agent`. When replying to one, also notify the operating agent directly, or address the reply to it.

## Auth & identity (per-agent tokens)

Contract: `docs/agent-tokens.md`.

- **Identity enforcement depends on a single `http` crate version shared by `axum` and `rmcp`.** `bearer_auth` puts `CallerClass` into the request extensions, rmcp hands that request's `http::request::Parts` to the tool context, and `helpers::bound_agent` downcasts it by `TypeId`. With two `http` versions the downcast misses silently and every caller looks unbound. After a dependency change `cargo tree -i http` must show one version; the identity-mismatch tests in `tests/integration.rs` cover the whole path.
- **Renaming an agent is a data cutover, not a code change.** Slugs are free-form, so no deploy renames anyone. Rebind `from_agent` in static tokens, add the new name next to the old one in machine-token `agents` allowlists until producers flip, and rewrite `handoffs.from_agent`/`to_agent`/`accepted_by` + `knowledge.author`. Tell the seat before the rewrite: its old slug otherwise gets an empty queue.
- **A bulk `UPDATE` on `handoffs` or `knowledge` re-ages the whole history.** `trg_handoffs_updated_at` / `trg_knowledge_updated_at` stamp `updated_at = now()` on every touched row, which resets briefing ages and stuck detection. Wrap data fixes in `ALTER TABLE … DISABLE TRIGGER …` / `ENABLE TRIGGER …` inside one transaction, and check that max `updated_at` is unchanged afterwards.
- **Changing family-token (`-*`) behaviour breaks Grok Bots silently.** `accept_handoff` requires `agent_name` for family credentials, and a seat that omits it fails every claim with nothing alerting. `family_accepts_require_explicit_concrete_identity` (`tests/handoff_reliability.rs`) now pins the rule and its error text. Change one only with the other, and tell family seats before deploying.
- **rmcp's `Extension<T>` extractor errors when `T` is absent; take the `Extensions` bag instead.** `T` is absent on every stdio call and under `--dev-no-auth`. Handlers that read caller identity take `ext: rmcp::model::Extensions` and treat absence as unbound. On the HTTP listener `bearer_auth` always inserts a `CallerClass` first.

## Handoff threads and IDs

- **A truncated handoff ID is ambiguous — thread on the full UUID.** IDs are UUIDv7, so handoffs filed seconds apart share their leading hex. A reply on the wrong thread fails quietly: `complete_handoff` on someone else's closed handoff returns `Handoff is already completed`, which reads like success. Keep full UUIDs in notes and commits, and after replying confirm the parent's `to_agent` is you.
- **Completing a parent does not complete its replies.** An `action` reply is independent open work. When closing a thread, complete each action reply explicitly, then run `check_in` once more to catch an orphan. `notify` replies need no cleanup.

## GitHub PR Workflow

- **`main` is protected: the four CI checks must pass (Format + Lint + Test, Security Audit, Shellcheck, Fleet-private string guard); no review is required, and admins can bypass.** A merge before checks finish is refused, not queued. Renaming a CI job silently drops it from the required set; update the protection with it.
- **Every agent commits as the same GitHub account, so none can approve another's PR.** `gh pr review --approve` fails with `Can not approve your own pull request`. Post the review with `gh pr comment`. Do not add a required-reviewer rule to this repo; it would block every merge.
- **A PR that conflicts with its base runs no checks at all.** `pull_request` workflows build `refs/pull/N/merge`, which does not exist while the branch conflicts, so nothing runs — the string guard included — and the page looks merely queued. Rebase first, then read the checks.
- **`gh run view <id>` outside a checkout of this repo fails with `failed to determine base repo`.** Pass `--repo TheK3nsai/ops-brain`. A watcher that treats that first failure as a dead release reports a running or green run as failed.
- **A stacked PR runs no checks either.** `ci.yml` and `secret-scan.yml` trigger on `pull_request: branches: [main]`, so a PR based on another branch matches nothing and still shows `mergeStateStatus: CLEAN`. Never merge it on the strength of the base's checks. Wait for the retarget to `main` to produce its own run, and meanwhile run the guard and the relevant suites by hand.
- **Deleting a stacked PR's base branch can close the stacked PR.** `gh pr merge --delete-branch` on the base did exactly that. A closed PR whose base ref is gone can be neither reopened nor retargeted. Since 2026-10-21 the repo auto-deletes head branches on merge, so leaving the flag off no longer keeps the base alive. GitHub documents that its own post-merge delete retargets dependent PRs, but that hasn't been observed here. **Retarget the child to `main` before merging its base** (`gh pr edit <child> --base main`). If one closes anyway, recover with `git rebase --onto main <old-base-tip> <branch>`, a force-push, and a new PR.
- **Editing a PR body does not retract it.** GitHub keeps every prior revision, readable by anyone on a public repo; the same holds for issue and comment bodies:

  ```
  gh api graphql -f query='{ repository(owner:"OWNER",name:"REPO") {
    pullRequest(number:N) { userContentEdits(first:20){ totalCount nodes{ editedAt deletedAt diff } } } } }'
  ```

  Treat a string that was ever in a PR body as published. A force-push does not unpublish the old commit either, and its blobs are served separately by the blobs API, so a cleanup request must name both. Verify reachability against a control: a fabricated SHA returns 422 for commits and 404 for blobs.
- **After a squash merge, "not on any remote branch" says nothing about whether the content shipped.** Squashing rewrites SHAs, so `git branch -r --contains` and `git cherry` report shipped work as unpushed, and genuinely unpushed commits look the same. Before resetting a diverged branch, compare content: `git diff --stat <local-branch> <squash-commit>` — an empty diff means it shipped. Tag first (`git tag rescue/<branch>-<date>`).
- **Local `main` drifts by default, because this repo squash-merges.** `--delete-branch` drops you onto a local `main` that still holds pre-squash commits. Fast-forward or reset it after every merge.
- **Cancelling a duplicate Actions run can turn a green PR red.** Close-and-reopen starts a second check suite for the same commit, the PR rollup follows the newest run of each check, and a cancelled run counts as not green. Let the newest required checks finish.

## Release / Windows editing

- **The MCP Registry name is case-sensitive: `io.github.TheK3nsai/ops-brain`.** Publishing with any other casing fails, and the image's `io.modelcontextprotocol.server.name` label must equal `server.json`'s `name` byte for byte. The ghcr image path stays lowercase.
- **Glama checks over stdio and needs an absolute CMD.** It wraps the CMD with `mcp-proxy`. An HTTP transport times out, and a bare command name fails with `spawn ENOENT`. Config: `docs/listings.md`.
- **A deployment behind Cloudflare can reject Python's default `urllib` User-Agent with 403 and `error code: 1010`.** The server never sees the request, so it reads like an auth failure. Send an explicit `User-Agent`, as `scripts/mcp-smoke.py` does; check with `/health`, which needs no bearer.
- **Pushing a `.github/workflows/` change needs a token with `workflow` scope.** Without it, GitHub rejects the push, not the PR. Push workflow edits from a checkout whose `gh` login has the scope.
- **A CHANGELOG version does not prove a release artifact exists.** Some versions have changelog sections and no tag. Check `git tag` or GitHub Releases before telling anyone to deploy a named release or image.
- **`sed -i` on a CRLF file under Git Bash rewrites it to LF, and MSYS tools hide it.** Afterwards `grep -c $'\r$'` still reports CRLF, and with `core.autocrlf=true` `git diff --stat` is clean. Only a byte count taken outside MSYS is trustworthy (`[IO.File]::ReadAllBytes($p).Length`). Edit CRLF files from PowerShell.
