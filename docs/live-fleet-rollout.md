# Live messaging fleet rollout

This runbook installs the private Claude Code and Codex live adapters on active
fleet hosts. It changes client launch behavior only: the production
ops-brain server already serves `/live`, and no database migration or server
restart is part of this rollout.

Live messaging is an attended, online-only lane. Do not install either adapter
as a service, scheduled task, login daemon, or wake-session dependency. A peer
is present only while its foreground Claude or Codex session is running.

**"Attended" is a mechanism, not a preference.** Both clients fail closed
without a real terminal, and they fail late. `codex --remote` exits with
`Error: stdin is not a terminal`, after which the Codex adapter blocks on
`waiting for exactly one loaded Codex thread; found 0` — registration is gated
on a genuinely loaded Codex thread, not on the adapter process being up. The
confusing part is that everything upstream succeeds first: the App Server
starts and `readyz` passes, so a headless attempt looks like it is working
right up to the moment it is not, and the failure reads as a broken adapter
rather than a missing TTY. Do not script, cron, or service-wrap this lane.

## Fleet matrix

| Host class | Platform | Claude identity | Codex identity | Launcher family |
|---|---|---|---|---|
| Linux host | Linux | `CC-<host>` | `Codex-<host>` | Bash |
| Windows host | Windows | `CC-<host>` | `Codex-<host>` | PowerShell |

Every row uses two different per-agent credentials. Never point both client
launchers at one token, reuse the main bearer, or copy a token between hosts.
The server-side binding is the source of peer identity.

## Common prerequisites

1. Pull an ops-brain revision containing the live adapters and launchers.
2. Install Node.js 22 or newer, current Claude Code with Channels support, and
   current Codex CLI with App Server/`--remote` support.
   **Do not verify Channels support with `claude --help`.** The
   `--dangerously-load-development-channels` flag is hidden: it is absent from
   `--help` output while present in the bundle and fully functional. Claude Code
   **2.1.231** is known good, confirmed independently on two Linux hosts.
   Checking `--help` will make you conclude the client lacks support and chase
   an upgrade that changes nothing. To check positively, grep the installed
   bundle instead of the help text:

   ```bash
   grep -rasoh dangerously-load-development-channels \
     "$(dirname "$(readlink -f "$(command -v claude)")")" | wc -l
   ```

   A non-zero count means the client **ships the flag**. It does not mean a
   channel will bind — see the launcher defect below, which reproduces on
   2.1.231 and 2.1.241 alike with a non-zero count on both. Treat this as a
   prerequisite check, never as evidence the lane works. On 2.1.231 this returns
   39 on both Linux hosts measured. The check is the principle, not the
   one-liner: recursively search the installed Claude Code bundle directory for
   the literal flag string. On Windows, run the equivalent recursive search
   (`Get-ChildItem -Recurse | Select-String`) against the install directory
   resolved from `Get-Command claude`; the flag being hidden from `--help` is a
   property of the client, not of the platform. The first measured Windows 11
   host returned 11 hits rather than the 39 seen on Linux; the exact count can
   vary with the installed bundle, and only a non-zero result is significant.
   On Windows, resolve and invoke the vendored native `codex.exe` directly; do
   not rely on the `codex` shim on PATH. **This is a path choice, not an install
   step.** `codex` on PATH is typically the npm `codex.cmd` → `node bin/codex.js`
   shim, but the same npm package already vendors the native binary:

   ```
   %APPDATA%\npm\node_modules\@openai\codex\node_modules\@openai\
     codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe
   ```

   Measured there at 285 MB / 0.147.0 on one Windows host, 2026-08-14. Wording
   that tells operators to *install* native codex sends them to a reinstall they
   do not need.

   Whether the shim can host the launcher's owned, log-redirected App Server
   process is **unmeasured**. A static read of `bin/codex.js` shows it resolves
   the vendored binary and spawns it with `stdio: "inherit"`, forwarding
   SIGINT/SIGTERM/SIGHUP and mirroring the exit code — so real console handles
   are inherited straight through to the native process. That rules out the
   TTY-loss failure the shim's presence would otherwise suggest, but does not
   settle whether the App Server keys on the launching process rather than the
   console owner. Use the native path regardless: it costs nothing and avoids
   the process-identity trap in gate step 9.
3. Confirm the host already has the two identity-bound agent tokens. Token
   delivery, minting, and rotation are attended credential operations and are
   outside this runbook.
4. Obtain the deployment's exact public `wss://<ops-brain-host>/live`
   endpoint from local operator configuration.

The adapter packages are private and lockfile-pinned. Install from the repo;
do not publish or install similarly named public npm packages.

## Linux installation and launch

From the ops-brain checkout:

```bash
scripts/install-live-adapters
scripts/install-live-adapters --status
```

The installer runs `npm ci --ignore-scripts` for both adapters and links the
two repo-owned foreground wrappers into `~/.local/bin`. It never reads or
copies credentials. Its status output checks both pinned dependency trees as
well as the launcher links, so `missing-or-invalid` is a failed prerequisite,
not a reason to attempt a live launch. If `~/.local/bin` is not in `PATH`,
invoke the wrappers by their repo paths or add the directory through the
host's normal dotfiles.

Launch Claude with the exact Claude token file:

```bash
OPS_BRAIN_LIVE_URL=wss://ops-brain.example/live \
OPS_BRAIN_AGENT_TOKEN_FILE="$HOME/.config/ops-brain/agent-token-cc-example" \
OPS_BRAIN_EXPECTED_AGENT=CC-Example \
OPS_BRAIN_LIVE_LABEL=claude-example \
ops-brain-claude-live
```

Launch Codex with the distinct Codex token file:

```bash
OPS_BRAIN_LIVE_URL=wss://ops-brain.example/live \
OPS_BRAIN_AGENT_TOKEN_FILE="$HOME/.config/ops-brain/agent-token-codex-example" \
OPS_BRAIN_EXPECTED_AGENT=Codex-Example \
OPS_BRAIN_CODEX_LABEL=codex-example \
ops-brain-codex-live
```

### KNOWN DEFECT: the Claude launcher's Channel does not bind (2026-08-24)

`ops-brain-claude-live` is **not usable as written**. It passes the adapter with
`--mcp-config` and references it as
`--dangerously-load-development-channels server:ops-brain-live`. The Channel
name resolver enumerates only the `enterprise`, `user`, `project`, and `local`
config scopes and checks the name exists in one of them. A server supplied on
the command line is in none of them, so launch prints:

```
server:ops-brain-live · no MCP server configured with that name
```

`/mcp` still shows the server **connected** — `--mcp-config` loads it fine. Only
the resolver uses the narrower lookup. **This is not version drift:** the same
function is byte-identical in 2.1.231 and 2.1.241. Whether passing
`--mcp-config` a *file path* rather than a JSON string changes anything is
**unmeasured**; the Windows launcher uses the file form, so its status is
unmeasured too, not known-good.

`ops-brain-claude-live --status` now reports this directly as a `channel:` line.
Check it before launching.

The gate workaround was registering the same definition in `local` scope and
launching without `--mcp-config`. **Do not ship that as the fix.** Scope
registration is ambient: every Claude session in that scope spawns an adapter
and claims the identity, so two overlapping sessions produce two peers under one
identity and make every send ambiguous — the exact condition step 5 exists to
catch. Hosts running a wake shim would hit it routinely. The real fix needs
resolution without ambient registration; tracked in `TODO.md`.

Both wrappers also accept `--status` and `--dry-run`. The token path and exact
expected server-bound identity are required deliberately. There is no generic
fallback that could silently bind a sibling identity, and registration fails
closed if the server reports another slug. Linux token files must be mode 600
or 400.

## Windows credential preparation

The Windows launchers require an existing agent token in a DPAPI-protected
`PSCredential` CliXml file. Create one file per identity in an attended host
session. The operator should paste the token into `Read-Host`; never put it in
the command line, transcript, environment, or this repository:

```powershell
$identity = 'CC-Example' # select the exact identity for this file
[IO.Directory]::CreateDirectory("$HOME\.secrets") | Out-Null
$secret = Read-Host "Agent token for $identity" -AsSecureString
$credential = [PSCredential]::new($identity, $secret)
$credential | Export-Clixml -LiteralPath "$HOME\.secrets\ops-brain-$identity.cred.xml"
$secret = $null
$credential = $null
```

Repeat with the host's Codex identity. DPAPI binds the encrypted value to that
Windows account and machine. This reduces ambient secret propagation; it is
not a security boundary against processes already running as the same user.

## Windows installation and launch

From a PowerShell 7.4+ prompt in the ops-brain checkout. The Windows bundle
requires .NET 8 because the Codex launcher redirects helper output while hiding
the child windows; older runtimes ignore the hidden window style in that mode.

```powershell
pwsh -NoProfile -File .\scripts\Install-OpsBrainLive.ps1
pwsh -NoProfile -File .\scripts\Install-OpsBrainLive.ps1 -Mode Status
```

The installer runs pinned `npm ci --ignore-scripts` and creates two `.cmd`
shims under `%LOCALAPPDATA%\Programs\ops-brain-live`. It reports when that
directory still needs to be added to the user's `PATH`.

Claude:

```powershell
ops-brain-claude-live `
  -LiveUrl 'wss://ops-brain.example/live' `
  -AgentCredentialFile "$HOME\.secrets\ops-brain-CC-Example.cred.xml" `
  -AgentName 'CC-Example' `
  -Label 'claude-example'
```

Codex:

```powershell
ops-brain-codex-live `
  -LiveUrl 'wss://ops-brain.example/live' `
  -AgentCredentialFile "$HOME\.secrets\ops-brain-Codex-Example.cred.xml" `
  -AgentName 'Codex-Example' `
  -Label 'codex-example'
```

Substitute the host's exact identities and labels. `-Mode Status` and
`-Mode DryRun` are credential-safe. The credential helper decrypts the bearer
only in the adapter child process. It verifies that the DPAPI credential
username matches `-AgentName`, and the adapter independently verifies the
server-returned binding. Claude, Codex, command arguments, generated MCP
configuration, and logs receive only the credential-file path.

## Capturing launcher output

Never pipe a launcher's stdout. `ops-brain-claude-live | tee rollout.log` makes
Claude Code see a non-TTY stdout and silently switch to `--print` mode, which
then dies with `Input must be provided either through stdin or as a prompt
argument when using --print`. This is the trap in recording a rollout receipt:
piping to `tee` is the obvious way to capture one and is exactly what breaks
the launch.

Use a recorder that keeps a real terminal in front of the client:

```bash
OPS_BRAIN_LIVE_URL=wss://ops-brain.example/live \
OPS_BRAIN_AGENT_TOKEN_FILE="$HOME/.config/ops-brain/agent-token-cc-example" \
OPS_BRAIN_EXPECTED_AGENT=CC-Example \
OPS_BRAIN_LIVE_LABEL=claude-example \
script -q -c ops-brain-claude-live rollout.log
```

`tmux pipe-pane` records the same way, but do not assume tmux is available for
this: on at least one fleet host the tmux *server* itself dies (`server exited
unexpectedly`) while hosting the Claude TUI. Prefer `script`; treat tmux as the
fallback.

The same rule applies to the PowerShell launchers — record with
`Start-Transcript` rather than piping to `Tee-Object`. That mechanism is
inferred from the Linux measurement, not yet measured on Windows.

## Per-host acceptance gate

Do not call a host live until all applicable checks pass:

1. Installer status reports Node, npm, and the client binaries; both adapter
   package installs complete without lifecycle scripts.
2. Each launcher status names the intended credential path and a non-sensitive
   label. Dry-run output contains no bearer.
3. Start only Claude. `list_live_peers` from its bound MCP token shows one
   `claude_code` peer with the exact Claude fleet identity — **and then deliver
   one marker to it and confirm the text actually appears in the session.**

   **A registered peer is not a receiving peer, and the difference is invisible
   from the bus.** The adapter connects and registers on startup, independent of
   the client's MCP handshake and independent of whether the Channel ever bound.
   A peer with the right identity, right label, and a `host_accepted` receipt can
   sit in front of a session that cannot receive anything. Measured on stealth
   2026-08-24: a healthy-looking `CC-Stealth` peer, attached to a session whose
   Channel had failed to resolve.

   This is how the Claude launcher defect below survived a documented
   "confirmed on two Linux hosts" — the two Claude-side checks were a bundle
   grep proving the flag exists, and a peer-presence assertion. Neither has ever
   tested delivery. Peer presence is a prerequisite, not the check.
4. Stop Claude and confirm the peer disappears. Repeat for Codex.
5. Start both, then confirm `list_live_peers` reports **exactly one** peer per
   identity before sending anything. `send_live_message` requires exactly one
   connected local adapter per bound agent to attribute source provenance; a
   stale adapter left by an earlier attempt leaves two peers under one identity
   and makes every send ambiguous. Nothing else surfaces this, and a failed
   earlier attempt is precisely when a stale peer exists — so check here rather
   than diagnosing a later send failure as a server problem.
6. Send one unique marker Claude -> Codex and a correlated reply
   Codex -> Claude. A `host_accepted` receipt means host injection accepted the
   text, not that the model read or followed it.
7. Run both halves of the identity negative control. They exercise different
   enforcement points, so passing one does not imply the other:
   - **7a, rendered provenance:** send harmless text that claims the sibling's
     `from_agent` and confirm the delivered envelope still renders the sender's
     server-bound identity.
   - **7b, credential claim:** cross exactly one field at a time: keep the
     launcher's own credential file but pass the sibling `-AgentName`, then
     repeat in the other direction. Confirm each mismatch is rejected before
     reading or transmitting the bearer. Passing the sibling name *and* sibling
     credential together is a valid matching pair and is not a negative
     control. Use only identity metadata in the receipt; never print a token or
     send an actual credential as test content.
8. Leave both sessions idle for at least three minutes through the production
   proxy, then repeat one marker exchange.
9. Exit both clients and confirm their peers disappear promptly and no owned
   App Server, adapter, or launcher process remains. Give a remote observer an
   explicit start signal or record an independent host-side exit timestamp
   before it begins timing. A poll taken before the client actually exits is a
   valid measurement of the wrong interval and can falsely report a stale peer.

   On Windows that host-side timestamp must be the exit of the **native
   `codex.exe` PID, obtained after launch** — not the PID the launcher returns.
   Through the npm shim the process tree is `cmd.exe` → `node.exe` →
   `codex.exe`, one level deeper again through a local wrapper `.cmd`, so a
   watcher built the obvious way (poll the PID from `Start-Process codex`)
   stamps **node's** exit: a distinct and later event than the client's. That is
   the same class of clock-ordering artifact as the unsynchronized start signal
   above, but sourced from process identity, so it will not look the same in the
   log — the interval reads as clean while bounding the wrong process.

Record only commit, versions, peer identities, receipt outcomes, timestamps,
and sanitized marker IDs in the rollout handoff. Never record token values,
credential contents, message bodies containing operational data, PII, or PHI.

## Rollback

Exit the foreground client and use the ordinary `claude` or `codex` launcher.
Live peers disappear automatically; durable MCP and handoffs are unaffected.
Uninstalling consists only of removing the two user launch shims/links and the
adapter `node_modules` directories. Do not delete or rotate credentials merely
to disable live messaging.
