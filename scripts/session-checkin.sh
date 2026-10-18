#!/usr/bin/env bash
#
# session-checkin.sh — print an agent's open ops-brain queue at session start.
#
# For Claude Code and Codex SessionStart hooks: both add a hook's stdout to the
# session context, so the agent starts knowing its waiting work without having
# to remember check_in. Polls GET /api/pending with a read-scoped machine token
# (docs/machine-callers.md). Always exits 0: a hook must never block a session.
#
# Env: OPS_BRAIN_URL (required), OPS_BRAIN_AGENT (required, the agent's slug),
#      OPS_BRAIN_PENDING_TOKEN_FILE (default ~/.config/ops-brain/pending-token).
# Needs curl and jq.

set -uo pipefail

url="${OPS_BRAIN_URL:-}"
agent="${OPS_BRAIN_AGENT:-}"
token_file="${OPS_BRAIN_PENDING_TOKEN_FILE:-$HOME/.config/ops-brain/pending-token}"

say() { printf 'ops-brain: %s\n' "$1"; exit 0; }

[[ -n $url && -n $agent ]] || say "session check-in not configured (set OPS_BRAIN_URL and OPS_BRAIN_AGENT)"
token=$(cat "$token_file" 2>/dev/null) || say "no token at $token_file; run check_in manually"
[[ -n $token && $token != *[\"\\$'\r\n']* ]] || say "token file is empty or malformed; run check_in manually"

# Bearer via --config on a process-substitution fd, never in argv.
response=$(curl -sf --max-time 10 \
    --config <(printf 'header = "Authorization: Bearer %s"\n' "$token") \
    "${url%/}/api/pending?agent=$agent&limit=20") || say "bus unreachable; run check_in manually"

jq -r --arg agent "$agent" '
  if (.count | type) != "number" then "ops-brain: unexpected response; run check_in manually"
  elif .count == 0 then "ops-brain: no open action handoffs for \($agent); check_in also shows replies and FYIs."
  else "ops-brain: \(.count) open action handoff(s) for \($agent)\(if .has_more then " (more not shown)" else "" end). Run check_in, then get_handoff for each:",
       (.items[] | "- [\(.priority)] \(.title) (from \(.from_agent), \(.status), id \(.id))")
  end' <<<"$response" 2>/dev/null || say "unexpected response; run check_in manually"
exit 0
