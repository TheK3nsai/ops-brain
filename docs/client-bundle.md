# ops-brain client bundle

This archive adds ops-brain's online delivery lane to foreground Claude Code
and Codex sessions. It contains both host adapters, their lockfile-pinned Node
dependencies, Linux and Windows launchers, and a credential-safe profile and
doctor command. No Git checkout or `npm install` is required.

Before extracting a downloaded release archive, verify its checksum and GitHub
artifact attestation:

```bash
sha256sum --check ops-brain-client-X.Y.Z.SHA256SUMS
gh attestation verify ops-brain-client-X.Y.Z.tar.gz --repo TheK3nsai/ops-brain
```

Use the `.zip` filename in the second command on Windows. The checksum detects
transfer corruption; the attestation binds the artifact digest to this
repository's GitHub Actions release workflow.

Keep the extracted versioned directory in a stable user-owned location. On
Linux, run:

```bash
scripts/install-ops-brain-client
ops-brain-client configure claude \
  --live-url wss://ops-brain.example/live \
  --agent CC-Example \
  --credential-file "$HOME/.config/ops-brain/agent-token-cc-example"
ops-brain-client configure codex \
  --live-url wss://ops-brain.example/live \
  --agent Codex-Example \
  --credential-file "$HOME/.config/ops-brain/agent-token-codex-example"
ops-brain-client doctor
```

Then launch `ops-brain-claude` or `ops-brain-codex`. The legacy command names
ending in `-live` remain aliases for compatibility.

On Windows, run `Install-OpsBrain.ps1`, configure profiles with the bundled
`ops-brain-client` command, and launch the installed `.cmd` shims. Windows
credential files must be DPAPI-protected `PSCredential` CliXml files as
described in `docs/live-fleet-rollout.md`; the profile stores only their path.

Online messages remain best-effort and untrusted. Use a handoff whenever the
target is offline or the work needs a durable lifecycle.
