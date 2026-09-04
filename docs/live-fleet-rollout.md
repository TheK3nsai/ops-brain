# Live messaging fleet rollout

This runbook installs the Claude Code and Codex online-delivery adapters on active
fleet hosts. It changes client launch behavior only: the production
ops-brain server already serves `/live`, and no database migration or server
restart is part of this rollout.

Live messaging is an attended, online-only lane. Do not install either adapter
as a service, scheduled task, login daemon, or wake-session dependency. A peer
is present only while its foreground Claude or Codex session is running.

**"Attended" is a mechanism, not a preference.** Both clients fail closed
without a real terminal, and they fail late. `codex --remote` exits with
`Error: stdin is not a terminal`, after which the Codex adapter blocks on
`waiting for exactly one resumable Codex thread; found 0 among 0 loaded` —
registration is gated on a genuinely resumable Codex thread, not on the adapter
process being up. The
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

1. Download and verify the versioned client bundle attached to an ops-brain
   release, or use a clean, pinned source checkout explicitly approved for the
   deployed server. Client and server revisions need not be identical; record
   both independently so a later checkout cannot blur the measured pairing.
2. Install Node.js 22 or newer, current Claude Code with Channels support, and
   Codex CLI with App Server/`--remote` support. The complete attended
   rendered-delivery gates passed with Codex **0.151.0 on Windows** using the
   published v5.2.1 client bundle (`02bd845`), then **0.152.0 on Linux** using
   source checkout `279ba8c` against the v5.2.1 server. Versions 0.149.0 and
   0.149.1 are earlier measured working versions.
   The adapter's resumable-thread selection handles the process-wide extra
   loaded ID first observed in 0.150.0, but 0.150.0 itself has not run the
   complete gate. Do not downgrade a current acceptance host merely to prove
   that historical point release, and do not generalize one measured version
   into an open-ended `>=` claim. Pin each gate to a declared client version and
   record `codex --version` with the receipt so a client change cannot erase the
   comparison baseline.
   **Do not verify Channels support with `claude --help`.** The
   `--dangerously-load-development-channels` flag is hidden: it is absent from
   `--help` output while present in the bundle and fully functional. Claude Code
   **2.1.257** passed the complete attended gate with the private configuration
   overlay on both Linux and Windows; 2.1.241 is the earlier Linux baseline.
   Checking `--help` will make you conclude the client lacks support and chase
   an upgrade that changes nothing. The integrated main-launcher path was
   operator-confirmed on Linux 2026-09-03 with Claude Code **2.1.259** and
   Codex **0.153.0**; same rule, that is a measured pair, not a `>=` claim. To check positively, grep the installed
   bundle instead of the help text:

   ```bash
   grep -rasoh dangerously-load-development-channels \
     "$(dirname "$(readlink -f "$(command -v claude)")")" | wc -l
   ```

   A non-zero count means the client **ships the flag**. It does not mean a
   channel will bind: direct `--mcp-config` registration fails the Channel
   resolver on both 2.1.231 and 2.1.241, while the launcher's isolated user-scope
   overlay binds on 2.1.241. Treat this as a prerequisite check, never as
   evidence the lane works. On 2.1.231 this returns 39 on both Linux hosts
   measured. The check is the principle, not the
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

From the extracted client bundle or an ops-brain checkout:

```bash
scripts/install-ops-brain-client
scripts/install-ops-brain-client --status
```

Release bundles already contain the lockfile-pinned dependencies; source
checkouts install them with `npm ci --ignore-scripts`. The installer links
`ops-brain-client`, `ops-brain-claude`, and `ops-brain-codex` into
`~/.local/bin`, plus the old `-live` command names as compatibility aliases. It
never reads or copies credentials. `missing-or-invalid` is a failed
prerequisite, not a reason to launch.

For a source checkout those commands are symlinks into that checkout, not
copies. A later branch switch or pull changes what the next invocation runs
without reinstalling anything. Pin the acceptance run to a clean checkout whose
tree matches the intended client revision, and do not change that checkout
between status/dry-run validation and teardown. A release bundle avoids this
source-tree drift because its extracted files are the installed client root.

Keep the **client revision** separate from the production server revision. A
rollout SHA in a handoff is a point-in-time instruction, not an evergreen alias
for whichever commit is deployed when the delayed run finally starts. If a
client release or blocker fix landed after the handoff was filed, resolve the
intended client revision on that thread before installing; do not guess whether
an old rollout SHA, a local rescue commit, or the current server checkout wins.
Prefer the newest explicitly approved release bundle, then record both client
and server revisions in the gate receipt.

Configure one protected profile per client. Profiles contain the credential
path, URL, exact identity, and non-sensitive label—never the bearer:

```bash
ops-brain-client configure claude \
  --live-url wss://ops-brain.example/live \
  --agent CC-Example \
  --credential-file "$HOME/.config/ops-brain/agent-token-cc-example" \
  --label claude-example
ops-brain-client configure codex \
  --live-url wss://ops-brain.example/live \
  --agent Codex-Example \
  --credential-file "$HOME/.config/ops-brain/agent-token-codex-example" \
  --label codex-example
ops-brain-client doctor
```

Launch the foreground sessions:

```bash
ops-brain-claude
ops-brain-codex
```

These launchers hydrate the live adapter from its protected credential file;
they do not hydrate the separate, regular `ops-brain` MCP server configured in
Claude or Codex. Before launch, confirm that any bearer environment variable
named by that ordinary MCP configuration is already present in the launcher's
process environment. A shell hydration *function* in `.bashrc` is insufficient:
the launcher is an executable script, so the function is not invoked. Claude
may still open with that MCP returning 401, but Codex treats a missing variable
for a required MCP server as a fatal TUI bootstrap failure. Hydrate only the
attended lane from its protected file; never make the bearer a global export or
put it in a command argument, transcript, or log.

### Reading the adapter logs

Both adapters write JSON lines to `OPS_BRAIN_LIVE_STATE_DIR`, default
`~/.local/state/ops-brain-live/`: `codex-adapter.*.log` and
`claude-adapter.*.log`. On Linux, each launcher's `--status` prints the path;
on Windows, use `-Mode Status`.

**On Claude this file is the only signal.** Claude Code spawns the adapter and
discards its stderr, so a live channel that never binds produces no terminal
output, and Claude Code's own MCP log records only its transport events —
`Successfully connected (transport: stdio)` refers to the stdio MCP transport,
**not** the `/live` WebSocket. An operator reading that line alone will
conclude the lane is working when it is not. Check the adapter log for
`live adapter connected` before believing a Claude session is bound, and treat
a `"retryable": false` record as terminal: the adapter has stopped and will not
recover without a configuration change.

On Windows, normal Claude Code exit can terminate the MCP child before Node
receives a socket-disconnect event or `SIGINT`/`SIGTERM`. The adapter log can
therefore end at `live adapter connected` even when the client, adapter, and
remote peer all tear down correctly. Absence of a terminal disconnect record is
not teardown evidence on that platform; use the native client exit timestamp,
owned-process check, and remote peer disappearance required by steps 4 and 9.

Each launch writes a new file. Since #115 both Linux launchers delete their own
adapter logs older than 30 days at launch; on Windows nothing prunes them, so
delete old files there as part of ordinary host maintenance. Records carry a
`ts` field because separate randomly named files have no other ordering.

### Claude Channel resolver isolation

Claude's Channel name resolver enumerates configured scopes but ignores servers
provided only through `--mcp-config`. Both inline JSON and file-form controls
fail with:

```
server:ops-brain-live · no MCP server configured with that name
```

The supported launcher creates a private per-launch `CLAUDE_CONFIG_DIR`, links
the user's non-config state into it without mutating the real files, and adds
the Channel only to the overlay's user scope. It loads an existing user MCP
document directly with `--mcp-config`; it never copies the complete
`.claude.json`. The resolver sees the internal Channel name; sibling and wake
sessions do not. The adapter waits for MCP initialization before registering
with `/live`, and the overlay is deleted when Claude exits.
Do not replace this with ambient local/user registration: every Claude session
in that scope would spawn the adapter and create duplicate peers.

Claude Code 2.1.257 on the measured Windows host treated the v5.2.1 overlay as a
first-run profile and repeated its onboarding prompts on each launch. The
current helper avoids that delay by carrying exactly two non-sensitive values
from the real config: one of Claude's six recognized theme names and a literal
`hasCompletedOnboarding: true`. It ignores false, malformed, or unknown values.
The Windows launcher harness verifies the allowlist; the next release gate must
confirm the prompts are gone in the real client.

**Know what the overlay carries.** `CLAUDE_CONFIG_DIR` *replaces* the user
config rather than layering over it, so the overlay's `.claude.json` contains
only the two allowlisted onboarding/display values and the generated
`ops-brain-live` Channel entry with a credential-file pointer. The real user MCP
document remains at its existing path and is passed to Claude as a file. The
launcher's own bearer is read by the adapter from the protected token file; it
is not copied into the overlay, exported by the launcher, or placed in a command
argument. On Linux the temporary overlay uses
`XDG_RUNTIME_DIR` when available and a private mode-700 `/tmp` directory
otherwise. Same-user processes remain outside this boundary.

On Linux, both launchers accept `--status`, `--dry-run`, and `--profile`. In
`--auto` none of them are the launcher's: `--auto` and `--no-live` are the
only switches it still reads there, and every other argument — including
`-h`/`--help`, `--`, and Codex's own `--profile` — goes to the client
untouched. Choose the ops-brain profile in `--auto` with
`OPS_BRAIN_CLAUDE_PROFILE` / `OPS_BRAIN_CODEX_PROFILE`. The
PowerShell launchers use `-Mode Status`, `-Mode DryRun`, and `-ProfileFile`
instead. The token path and exact expected server-bound identity are required
deliberately. There is no generic
fallback that could silently bind a sibling identity, and registration fails
closed if the server reports another slug. Linux token files must be mode 600
or 400.

## Main-launcher integration (Linux)

Once the explicit-launcher gate above is clean on a host, plain `claude` and
`codex` can become the live launchers for that host's interactive shells. The
mechanism is two shell functions, sourced from the shell's rc file:

```bash
. "<client-root>/scripts/ops-brain-shell-init.sh"
```

The installer prints the exact line for its checkout or bundle. Each function
runs the matching launcher in `--auto` mode and carries no credential: the
launcher still reads the protected token file itself, and nothing is exported.
Shell functions are not inherited by child processes, so scripts, systemd
timers, wake shims and anything else that execs `claude` or `codex` by name
reach the real binaries untouched.

`--auto` is the main-launcher contract. It goes live only for an attended
foreground TUI, and it treats every other shape as ordinary without a word:
a non-terminal stdin or stdout, `claude -p`/`--print`, `codex exec` and every
other subcommand, `--version` and `--help`. Those sessions cannot render a
channel event, and a peer with no sink is the "healthy session, absent lane"
defect the gate exists to catch. When live is wanted and preflight fails (no
profile, missing or badly-moded token, adapter absent), `--auto` prints the
reason on the terminal and asks:

```
ops-brain live: NOT available — <reason>
Continue without live delivery? [y/N]
```

Only an explicit `y` launches an ordinary session, and that launch is
announced. Anything else exits nonzero without launching. There is no silent
fallback in either direction, and the explicit `ops-brain-claude` /
`ops-brain-codex` commands keep failing closed with no prompt, so they remain
the rollout boundary. A deliberate ordinary launch is `claude --no-live` (or
`OPS_BRAIN_LIVE=off claude`), announced once on stderr; `command claude ...`
bypasses the function entirely.

A live launch prints one line before the client takes the terminal:

```
ops-brain live: connecting as CC-Example (label claude-example.<cwd>); adapter log: ...
```

The label now carries the working directory's basename, folded to the
label alphabet and bounded to 80 bytes. Several attended sessions may run
under one identity; each is its own peer, and the label is what a remote
sender uses to choose between them. Inside a session, the channel's
`list_live_peers` reports the session's own peer under `self`. If the adapter
later stops for good (identity mismatch, terminal server rejection), it emits
one channel event marked `kind=lane_status` so the session itself learns the
lane is gone rather than only the log; the instructions tell Claude to relay
it in one line and not retry. On the Codex side, a second attended session
finds the profile's App Server port already owned by the first and takes a
free loopback port for its own App Server and adapter, so the adapter's
exactly-one-thread invariant holds per session.

Everything in the per-host acceptance gate applies unchanged to the
integrated path, and a host is not certified on it until the same
two-identity, rendered-marker, idle, and teardown checks pass launched from
the plain `claude` and `codex` commands. The Windows equivalent is
[below](#main-launcher-integration-windows).

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

From a PowerShell 7.4+ prompt in the extracted bundle or ops-brain checkout.
The Windows bundle requires .NET 8 because the Codex launcher redirects helper
output while hiding the child windows; older runtimes ignore the hidden window
style in that mode.

```powershell
pwsh -NoProfile -File .\scripts\Install-OpsBrain.ps1
pwsh -NoProfile -File .\scripts\Install-OpsBrain.ps1 -Mode Status
```

Release bundles contain pinned dependencies; source checkouts install them with
`npm ci --ignore-scripts`. The installer creates `ops-brain-client`,
`ops-brain-claude`, and `ops-brain-codex` `.cmd` shims under
`%LOCALAPPDATA%\Programs\ops-brain`, plus compatibility aliases. An upgraded
host that already has `%LOCALAPPDATA%\Programs\ops-brain-live` and does not yet
have the current directory keeps using that legacy location; installer status
is the source of truth for the effective path. It reports when the selected
directory still needs to be added to the user's `PATH`.

Configure profiles after creating the two DPAPI credentials:

```powershell
ops-brain-client configure claude `
  --live-url wss://ops-brain.example/live --agent CC-Example `
  --credential-file "$HOME\.secrets\ops-brain-CC-Example.cred.xml" `
  --label claude-example
ops-brain-client configure codex `
  --live-url wss://ops-brain.example/live --agent Codex-Example `
  --credential-file "$HOME\.secrets\ops-brain-Codex-Example.cred.xml" `
  --label codex-example
ops-brain-client doctor
```

Launch the foreground sessions:

```powershell
ops-brain-claude
ops-brain-codex
```

On a brand-new Codex TUI, submit one ordinary initial prompt before expecting a
live peer. The adapter deliberately waits—with bounded exponential backoff—until
exactly one process-wide loaded thread has persisted state and can be resumed.
An additional in-memory thread without a rollout is ignored; multiple persisted
candidates remain an ambiguity and keep the adapter offline. App Server readiness
alone is not peer readiness.

Substitute the host's exact identities and labels. `-Mode Status` and
`-Mode DryRun` are credential-safe. Before either client is spawned, a
validation-only helper invocation checks that the DPAPI credential username
matches `-AgentName` without reading the bearer. A second short-lived helper
invocation decrypts the bearer for the adapter and refuses to emit unless its
stdout handle is an operating-system pipe; consoles and file redirections fail
closed. The adapter captures that pipe without putting the bearer in its
environment, on disk, or on the command line, and independently verifies the
server-returned binding. Claude, Codex, command arguments, generated MCP
configuration, and logs receive only the credential-file path.

## Main-launcher integration (Windows)

The PowerShell launchers carry the Linux contract as `-Mode Auto`, and the
profile integration is one dot-source line in `$PROFILE`:

```powershell
. "<client-root>\scripts\OpsBrain-Shell.ps1"
```

The installer prints the exact line for its checkout or bundle, and
`Install-OpsBrain.ps1 -Mode Status` reports it as `shell init:`. The file
defines `claude` and `codex` functions only when the session is an attended
console: `ConsoleHost`, stdin and stdout not redirected, and pwsh not started
with `-NonInteractive`. A scheduled task, a wake shim, or any script that
dot-sources the profile gets no functions and reaches the real executables by
name. The functions carry no credential; the launchers still read the DPAPI
credential themselves. Bypass the function with
`& (Get-Command claude -CommandType Application) ...`.

Two measured Windows facts shape the mechanism, and they are why the functions
do not simply call the `.cmd` shims:

- `pwsh -File` binds a client's `-p` to the launcher's `-ProfileFile` by
  parameter-name prefix and eats `-v` as `-Verbose`; `--` is rejected in
  `-File` mode, and a splatted array after `--` is bound again. The one shape
  that carries every client argument intact is a single explicit array, so
  the functions run the launcher in-process as
  `ops-brain-claude-live.ps1 -Mode Auto -ClaudeArgs $args` (and `-CodexArgs`).
- `cmd.exe` re-parses `%*`: an unquoted `&` or `|` in a client argument splits
  the command line at the shim. In-process invocation has no shim in the path.

`-Mode Auto` goes live only for an attended console and treats every other
shape as ordinary without a word: redirected stdin or stdout, `-p`/`--print`,
`codex exec` and every other subcommand, the same Claude subcommand list as
the Linux launcher, `--version`/`-v`/`-V`, `--help`/`-h`. A failed preflight
on a console prints `ops-brain live: NOT available — <reason>` on stderr and
asks `Continue without live delivery? [y/N]` through `Read-Host`; only `y` (or
`yes`) launches an ordinary session, announced as `off (operator choice)`, and
anything else exits 2 with `declined ordinary fallback`. `-Mode Run` — the
explicit `ops-brain-claude`/`ops-brain-codex` commands — keeps failing closed
with no prompt. The deliberate opt-out is `claude --no-live` (a leading
`--no-live` client argument, the `-NoLive` switch, or `OPS_BRAIN_LIVE=off`),
announced once on stderr.

Labels carry the working-directory leaf (`<profile label>.<leaf>`, folded to
`[A-Za-z0-9._-]`, leading `.` stripped, bounded to 80 bytes) in Run, Auto and
DryRun; `-Mode DryRun` prints the resulting `label:` line. A second attended
Codex session that finds the profile port already answering `readyz` takes a
free loopback port for its own App Server and adapter and says so on stderr;
an explicit `-AppServerPort` or `OPS_BRAIN_CODEX_APP_SERVER_PORT` still fails
closed on a busy port. A live launch prints
`ops-brain live: connecting as <agent> (label <label>); adapter log: <path>`
before the client takes the console; for Codex the path is the adapter's
stderr log, where its structured lines go. Adapter and App Server logs older
than 30 days are pruned at launch.

`Test-OpsBrainLiveWindows.ps1` drives the headless shapes through a child
pwsh with piped handles, and the attended shapes — the live overlay launch,
subcommand passthrough, the decline/accept/empty-answer prompts, and the
profile functions — through a hidden child console fed by `WriteConsoleInput`.
When no console can be allocated it prints a warning that the attended paths
were not exercised instead of passing silently.

The per-host gate applies unchanged: a Windows host is not certified on the
integrated path until the two-identity, rendered-marker, idle, and teardown
checks pass launched from plain `claude` and `codex`, plus `claude -p` and
`codex exec` from the same session creating no peer, and a deliberately broken
preflight prompting and exiting 2 on `n`.

## Capturing launcher output

Never pipe a launcher's stdout. `ops-brain-claude | tee rollout.log` makes
Claude Code see a non-TTY stdout and silently switch to `--print` mode, which
then dies with `Input must be provided either through stdin or as a prompt
argument when using --print`. This is the trap in recording a rollout receipt:
piping to `tee` is the obvious way to capture one and is exactly what breaks
the launch.

Use a recorder that keeps a real terminal in front of the client:

```bash
script -q -c ops-brain-claude rollout.log
```

That receipt captures the foreground client, not background adapter diagnostics.
For Codex in particular, a healthy App Server and quiet launcher can coexist with
an offline peer. Run `ops-brain-codex --status` before launch, retain the printed
`codex-adapter.*.log` location with the receipt, and inspect the new JSONL file
if the peer does not appear. The decisive thread-selection warning is written
there and is not included in `script -q -c` output. Apply the same rule to the
Claude adapter log described above.

`tmux pipe-pane` records the same way, but do not assume tmux is available for
this: on at least one fleet host the tmux *server* itself dies (`server exited
unexpectedly`) while hosting the Claude TUI. Prefer `script`; treat tmux as the
fallback.

The same rule applies to the PowerShell launchers — record with
`Start-Transcript` rather than piping to `Tee-Object`. That mechanism is
inferred from the Linux measurement, not yet measured on Windows.

## Per-host acceptance gate

Do not call a host live until all applicable checks pass:

1. Installer status reports Node and the client binaries; both adapter
   dependency trees report `ready`. (`npm` is needed only when repairing a
   source checkout whose dependencies are absent or invalid.)
2. Each launcher status names the intended credential path and a non-sensitive
   label. Dry-run output contains no bearer.
3. Start only Claude. `list_live_peers` from its bound MCP token shows one
   `claude_code` peer with the exact Claude fleet identity.

   The Claude adapter now waits for MCP initialization before registering, and
   the isolated config overlay makes the Channel resolvable. Peer presence is
   still only a transport prerequisite. Do not attempt the delivered-marker
   control from that same identity-bound MCP session: the server correctly
   refuses a live peer messaging itself. Step 6 performs the rendered-delivery
   control once both distinct identities are connected.
4. Stop Claude and confirm the peer disappears. Repeat for Codex.

   **Launching both clients at once skips steps 3 and 4, and nothing later
   recovers them** — step 9 tears both down together, so neither adapter is
   ever observed registering or deregistering alone. If both are already
   running, make teardown sequential (exit one, poll, exit the other) to
   recover at least the per-adapter deregistration evidence, and record the
   rest as a deviation rather than a pass.
5. Start both, then confirm `list_live_peers` reports **exactly one** peer per
   identity before sending anything. `send_live_message` requires exactly one
   connected local adapter per bound agent to attribute source provenance; a
   stale adapter left by an earlier attempt leaves two peers under one identity
   and makes every send ambiguous. Nothing else surfaces this, and a failed
   earlier attempt is precisely when a stale peer exists — so check here rather
   than diagnosing a later send failure as a server problem.
6. Send one unique marker Claude -> Codex and a correlated reply
   Codex -> Claude. A `host_accepted` receipt means host injection accepted the
   text, not that the model read or followed it. Confirm that both markers
   actually render in their target sessions; this is the delivered-marker
   control for both adapters, not merely a receipt check.
7. Run both halves of the identity negative control. They exercise different
   enforcement points, so passing one does not imply the other:
   - **7a, rendered provenance:** send harmless text that claims the sibling's
     `from_agent` and confirm the delivered envelope still renders the sender's
     server-bound identity.
   - **7b, credential claim:** cross exactly one field at a time: keep the
     launcher's own credential file but pass the sibling agent name, then
     repeat in the other direction. Passing the sibling name *and* sibling
     credential together is a valid matching pair and is not a negative
     control. Use only identity metadata in the receipt; never print a token or
     send an actual credential as test content.

     **The enforcement point is platform-dependent, and the two are not
     equivalent.** On Windows the DPAPI `PSCredential` carries a username, so a
     validation-only helper compares it to `-AgentName` and rejects before the
     bearer is decrypted. On Linux the token file is an opaque bearer with no
     embedded identity, so no local pre-check is possible and
     `ops-brain-client configure` will accept a crossed profile without
     complaint. Linux enforcement is fail-closed at registration: the adapter
     authenticates to the configured endpoint, the server returns the token's
     bound slug, and the adapter refuses it and stops. Confirm no peer appears
     in either direction, and read the adapter log for
     `bound identity does not match`. Do not record the Linux result as
     "rejected before the bearer was transmitted" — it is not, and no
     configuration makes it so.
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
Uninstalling consists only of removing the installed client command links/shims
and the extracted client bundle (or adapter `node_modules` in a source
checkout). Do not delete or rotate credentials merely to disable online
delivery.
