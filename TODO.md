# TODO

Open work only. Shipped history lives in `CHANGELOG.md`, doctrine and hard stops in `ROADMAP.md`, code and deploy footguns in `GOTCHAS.md`.

**Release-intent doctrine, still in force:** _"don't sharpen for optics; build for the 4 CCs."_ The external-user release sweep shipped 2026-05-18 (#56, #57). No further external-polish work without actual external signal — someone files an issue, asks for help, or otherwise shows up.

## Open

- **Pre-scrub commit residue: CONFIRMED reachable, but the GC request is the wrong lever (measured 2026-08-24).** Re-checked the 2026-08-23 / PR #85 scrub against GitHub with positive *and* negative controls (a fabricated SHA 422s / 404s, so the 200s mean something). Three independent surfaces still serve the pre-scrub content: the commit object `a793cb7`, the `GOTCHAS.md` blob `6b2dc55` (reachable directly via the blobs API, independent of the commit), and the PR #85 body — GitHub retains 2 `userContentEdits` revisions with `deletedAt: null`, so the original body is still queryable over GraphQL. So "may be reachable" is now "is reachable, by three paths."
  **But GC would accomplish almost nothing.** The same class of fleet-private string is already in *merged* `main`, permanently, since 2026-04-26: `migrations/20260426000002_normalize_handoff_machine_names.sql` (6 matches — a migration that normalizes machine names necessarily enumerates them) plus the commit messages of `4580d19`, `8281d9d`, and `dd21f0d`. Garbage collection never touches merged history. Purging one dangling object while a merged migration publishes the same names is theatre. **Eduardo's call, and it is now a bigger question than a support ticket:** either accept the exposure fleet-wide and stop scrubbing case-by-case, or plan a history rewrite. Don't file the GC request as a standalone — it buys ~nothing on its own.
- **The guard publishes its own denylist (2026-08-24) — Eduardo's call.** `.github/workflows/secret-scan.yml` carries the fleet-private pattern as a plaintext literal, on a public repo. Anyone reading the guard gets the complete enumerated list of the strings it exists to protect — the file is the highest-signal place to look for exactly those values. Fix is straightforward (source the pattern from a repository secret / `${{ secrets.* }}` and drop the literal), but it changes the repo's security posture and the local pre-commit hook shares the list, so it wants a deliberate decision rather than a drive-by PR.
- **Guard gap, unchanged and now measured on both ends: it scans one surface of three.** The CI job greps added lines of `git diff BASE HEAD`; the local pre-commit hook greps `git diff --cached`. Neither ever reads a commit message or a PR body — which is precisely where two of the three PR #85 leaks landed, and why only the `GOTCHAS.md` line was caught. Also note the CI half runs *after* the push that publishes the string. If this recurs, extend the local hook to `COMMIT_EDITMSG` rather than leaning on CI.

### PR #84 — JSON error envelope for the REST surface — deployed, awaiting HSR re-probe

Merged to `main` as `922b329`, **deployed and live on ops.kensai.cloud
2026-08-24** (CC-Cloud, handoff `01a03461`; rebuild only, `serverInfo.version`
5.0.0, container healthy). Production now returns `application/json` on every
`/api` rejection as `{"error": "...", "field": "..."}`, with `field` present
only when the rejection is attributable to one input field.

CC-Cloud probed seven cases on the deployed surface — both 401 paths paired
with a 200 positive control on the same URL, plus `400 field=agent`,
`400 field=since`, `403 field=agent`, and a scope 403. Envelope behaves as
specified throughout.

**The re-probe ping CC-Stealth owed CC-HSR was sent 2026-08-24** (`01a03463`,
on thread `01a0206b`): full byte-boundary table re-run against production,
200 OK positive control included, response bodies for the failing rows.
Non-mutating — validation rejects ahead of any insert.

**Only open item: HSR's re-probe results.** Close this section when they land.
A `text/plain`, bodyless, or wrong-`field` result is a real regression and goes
back to CC-Cloud immediately.

**The oversized-title path is the one case not live-probed from kensai-cloud,
and that is the scope control working, not a coverage hole.** Scope is enforced
in `required_machine_scope` middleware ahead of the handler, and the only
machine token on that host is `read` scope — so `POST /api/handoff` 403s before
it ever reaches `validate_bounded_text`. HSR-HVFS0 holds `create` scope and is
the actual bitten caller, which is why their probe is the real end-to-end check.
**Decided 2026-08-24: do not spend the break-glass bearer to close this from the
cloud side** — every use of `CallerClass::Full` is an exposure event, reaching
for a stronger credential to cross a scope boundary is the instinct to distrust,
and a `Full`-class probe would be weaker evidence than HSR's real-caller probe
anyway. Test coverage exists regardless
(`oversized_title_names_the_field_and_the_byte_count`, CI green on `922b329`).

Worth keeping: the reported symptom (*bodyless 400 on an oversized title*) was
**not reproducible** — production has always returned `title too large (N bytes,
max 200)`. The real defects were the *content-type* (`text/plain` on a JSON API,
so a JSON-parsing producer drops the body) and `bearer_auth`'s bare
`StatusCode`, which was genuinely bodyless. Client side, CC-HSR's producer lib
logged `$_.Exception.Message`, which on PowerShell never carries the response
body — fixed 2026-08-20 with 4 mutation-proven regression tests. The durable
lesson: **the wire fix cannot reach a client that isn't reading the right
property.**
- **Redesign the Claude launcher's Channel wiring — `ops-brain-claude-live` has never worked (2026-08-24).** `--mcp-config` servers are invisible to the Channel name resolver, which only enumerates the `enterprise`/`user`/`project`/`local` scopes; byte-identical in 2.1.231 and 2.1.241, so this is not version drift and no upgrade fixes it. Full mechanism in `GOTCHAS.md`; operator-facing note in `docs/live-fleet-rollout.md`; `--status` now reports it as a `channel:` line.
  **The design constraint is the whole problem:** scope registration is what makes the name resolve, and scope registration is *ambient* — every Claude session in that scope spawns an adapter and claims the identity, so two overlapping sessions produce duplicate same-identity peers and every send becomes ambiguous. That is the condition gate step 5 exists to catch, and hosts running a wake shim would hit it routinely. So the fix is neither "pass it differently" nor "register it in a scope"; it needs resolution *without* ambient registration. Candidate directions: launcher-owned register/launch/cleanup with a trap plus a refuse-if-already-registered guard (mutates `~/.claude.json`, which Claude Code also writes — race risk, needs thought), or an isolated per-launch scope directory. Not started; do not ship the local-scope workaround used during the gate.
  Unmeasured and worth settling first, because it may narrow the work: whether `--mcp-config` given a *file path* behaves differently from a JSON string. The Windows launcher uses the file form, so **Windows status is unmeasured, not known-good.** Test interactively with a bogus-name control — `claude -p` does not evaluate channels at all and will return a false pass.
- **Gate step 4 (solo appear/disappear per client) has never been run discretely.** The 2026-08-24 paired run passed 8 of 9 steps but substituted a strict step 5 for step 4. Step 5 is the stronger guard for stale adapters, but step 4 is still unproven on this host; run it on the next attended pass rather than carrying the gate forward as a clean sweep.

## Don't re-propose without new evidence

Deliberate decisions with their reasons. If real friction ever shows up, re-open the question from first principles — don't resurrect the design.

**Self-hosting the alert relay on the monitored box (2026-07-29).** When
picking a notification provider for Kuma, self-hosted ntfy behind the existing
Caddy looks like the tidy, no-third-party answer. It isn't: **it shares fate
with Caddy, and Caddy is one of the monitored services.** Caddy down, cert
expired, proxy misconfigured — the alert about it cannot get out, because it
routes through the thing that broke. That is the #77 defect wearing a different
hat, and the generalization is worth more than the specific call: *an alert
path must not traverse anything it is responsible for reporting on.*

Hosted ntfy.sh needs only outbound HTTPS from the container, so it survives
every partial failure short of host death — and host death takes Kuma with it
regardless of provider, which is why that gap is scoped out separately. It also
keeps the off-box-check door open: an external checker can publish to the same
topic, which a relay living on the monitored box forecloses.

The privacy tradeoff is bounded by *what we send*, not by a promise about ntfy:
monitor names and short status strings about our own services, no client data,
no PHI. If a monitor ever carries client-identifying content, re-run this
reasoning rather than inheriting it.

Pushover was the closest alternative — stable tokens, real auth rather than an
unguessable topic — and is the upgrade path if obscurity-based auth ever
bothers us. Declined because it reintroduces a credential to store and rotate,
and "a credential that died quietly" is the failure being fixed.

**Four-lens audit declines (2026-07-17, v4.1.0, PRs #64/#65/#67).** Every finding was fixed, tested, or consciously declined. The declines:

- **Verbatim DB error strings to callers** — every caller is one of our own authenticated agents; informative errors beat sanitization theater here.
- **`build_client_lookup` caching / backfill batching / embedding retry-backoff** — real, but irrelevant at 4-agent scale. The perf audit put these on its own "theater" list.
- **Unified error enum** — the current boundary-appropriate typing (`sqlx::Error` → `CallToolResult` / `(StatusCode, String)`) is deliberate, not mess.
- **Briefing cosmetic nits** — a section is omitted when a LIMIT-20 page holds none of a counted status; the unscoped `_note` yields to the rarer embedding note on multi-table. Edge-case cosmetic, flagged in #67's body.

**Automation backbone, phase 3 (2026-07-17, joint design with CC-HSR).** Phases 1+2 shipped; producer contract in `docs/machine-callers.md`. Deliberately not built: server-side recurrence (producers' own schedulers plus `dedupe_key` idempotency cover it), structured handoff columns (the versioned `context` convention instead — promote fields only if a pilot bleeds from prose-parsing), and webhooks-out (poll `GET /api/pending`; an additive upgrade if sub-minute latency ever becomes real pain).

**Operator visibility, tier 2 (2026-07-28).** A log or view of everything happening headless. The handoffs table already *is* the log — `origin`, `status`, `repeat_count`, threading, timestamps. The only real gap is that `updated_at` is destructive, so there's no `accepted_at` and "how long did this sit on a human" isn't answerable. That's a schema change in service of a metric nothing is bleeding from. If friction shows up, the cheapest next step is a section in the briefing that already gets read — not a web view, not event sourcing. Full reasoning in `docs/operator-notify.md`.

## Closed

- **Kuma's alerts reach no human** — reopened 2026-08-02, **closed 2026-08-03**. The 08-02 reopen was correct: the provider delivered to the ntfy topic (verified by polling the message back off ntfy.sh) while no device had ever subscribed, and ntfy's 200-for-any-topic makes those two states identical from the box. The operator subscribed on 08-03; the cloud host's `TODO.md` records the follow-on decisions taken "with alerting finally reaching a phone", and `kuma-watchdog.sh` was built on top of it. Closed here on that operator confirmation — **there is no server-side test that could close it**, which is the durable lesson (gotcha `019fc421-a935`, generalizing to any alert path terminating off-box). Fixed en route: `resend_interval = 0` on all 32 monitors, the Kuma default that alerts once on the DOWN transition and then goes silent forever — now ~6h via `max(1, round(21600 / interval))`, counting consecutive failed checks rather than minutes; mechanics in the cloud host's `docker/CLAUDE.md`. **Residual, tracked and deliberately open elsewhere:** nothing verifies the phone *stays* subscribed, so iOS delivery rot would silently return us to "alerts nowhere" with every server signal green. That is logged as an accepted gap in the cloud host's `TODO.md` (2026-08-03) — don't re-file it here as newly discovered.

- **The operator notifier shares fate with the channel it reports on** — 2026-07-29, #79, live the same day. Opened 07-28 during the #77 deploy: one revoked Gmail app password killed the briefing, the security digest, and operator-notify at once, so the handoff reporting "outbound ops email is dead" could not be delivered *because of the condition it was reporting*. Nothing was ever lost — a failed send holds the cursor and retries — but a dead notifier and a quiet bus looked identical. Resolved **without** the second credential the item was weighing: the escalation channel already existed and was already exercised daily (Uptime Kuma, which the sibling wake shim has pushed to since day one, and which owns monitoring by doctrine). `operator-notify.sh` gained an optional `$OPS_NOTIFY_HEARTBEAT_URL` it pings `up`/`down` on every real run — no Rust, no MCP surface, no failure counter, no outage-only path that could rot unnoticed. Thresholds and routing stay in the product that owns them. Control-tested 12/12 including unchanged cursor retry semantics and that an unreachable monitor is not a new failure mode. Kuma's side landed the same day: hosted ntfy.sh as a default-enabled provider, `Operator Notify` push monitor (1800s/1), crontab sourcing `conf/.env` so the push token stays out of a `ps`-visible command line. Setup gotchas — including the "Default enabled" backfill trap — in `docs/operator-notify.md`. **Correction (2026-08-02): "Kuma's side landed" overclaimed.** The provider was wired correctly and does deliver to the topic — verified — but no device was ever subscribed, so nothing reached a human. The ops-brain half of this item (`$OPS_NOTIFY_HEARTBEAT_URL`) is genuinely closed; the Kuma half was reopened 08-02 and closed 08-03 — see the entry above it. **Scope held:** this covers a dead *channel*, not a dead *host*; if the box dies, monitor and notifier die together, which wants an off-box check and is a different problem.

- **Operator visibility, tier 1** — 2026-07-28, #77. The design session resolved it to a convention plus a cron with **zero lines of Rust**: agents reply into the existing thread addressed to the operator's slug, and a read-scoped machine token plus a cron polls that queue and mails a digest. Contract in `docs/operator-notify.md`, poller in `scripts/operator-notify.sh`. Deployment and token mint handed to CC-Cloud (`019fa9fe`).
- **Per-agent MCP tokens with server-bound `from_agent`** — 2026-07-28, #73 and #75, released in v4.2.0. Deployed 2026-07-21; minting completed fleet-wide 2026-07-24 (prod boots `agent tokens configured count=8`), binding control-tested per host, and revoked tokens verified dead by probing for 401 rather than assumed. Remaining residuals are ops on other lanes: CC-HSR's local install of the re-minted pair (`019f95df`) and demoting the old shared bearer to break-glass.
- **Case-insensitive agent matching on handoff queries** — 2026-07-17, #64. `bb9a25b` had already made the agent filters `ILIKE`; the residual `_`-wildcard over-match on the MCP list filters is closed with `LOWER() = LOWER()` exact matching, plus an integration test pinning that `agent_x` no longer matches `agentXx`.
- **External-user release sweep** — 2026-05-18, #56 and #57. Repo description and topics, GitHub Releases backfill, README lede and quick-start, GHCR multi-arch pipeline, standalone `docker-compose.example.yml`. The `v3.2.0` image is live and public at `ghcr.io/thek3nsai/ops-brain`.
- **Node 20 → Node 24 action versions** — bumped to explicit Node-24 majors across `.github/workflows/` ahead of GitHub's 2026-06-02 auto-force.
