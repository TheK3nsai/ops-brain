#!/usr/bin/env bash
#
# operator-notify.sh — tell the operator when the bus is blocked on them.
#
# Polls GET /api/pending for handoffs addressed to a human slug (default
# "Operator") with a read-scoped machine token, and renders a digest of
# anything new since the last run. Sending is delegated: the digest goes to
# $OPS_NOTIFY_MAIL_CMD on stdin, or to this script's stdout if that is unset.
#
# This is the reference implementation of docs/operator-notify.md. It adds no
# ops-brain surface — the endpoint, the token scoping, and the cursor
# semantics all already exist for the wake shim; this points them at a human.
#
# Exit codes: 0 = nothing to send, or sent. 1 = poll/send failed (cursor is
# NOT advanced, so the next run retries the same items).

set -euo pipefail

OPS_URL="${OPS_BRAIN_URL:-}"
AGENT="${OPS_NOTIFY_AGENT:-Operator}"
TOKEN_FILE="${OPS_NOTIFY_TOKEN_FILE:-$HOME/.config/ops-brain/operator-notify-token}"
STATE_DIR="${OPS_NOTIFY_STATE_DIR:-$HOME/.local/state/operator-notify}"
CURSOR_FILE="$STATE_DIR/since"
LOG_FILE="$STATE_DIR/notify.log"
MAIL_CMD="${OPS_NOTIFY_MAIL_CMD:-}"

usage() {
    cat <<'EOF'
Usage: operator-notify.sh [--dry-run|--status|--reset]

Polls ops-brain for handoffs addressed to the operator and emits a digest of
anything new since the last run. Intended for cron (every 15-30 min).

Options:
  --dry-run   Poll and print the digest; never send, never advance the cursor
  --status    Show config + cursor state and exit
  --reset     Clear the cursor (next run re-reports every open item)
  -h, --help  Show this help

Environment:
  OPS_BRAIN_URL            base URL of your ops-brain deployment (required)
  OPS_NOTIFY_AGENT         human slug to poll for (default Operator)
  OPS_NOTIFY_TOKEN_FILE    read-scoped machine token, mode 600 (required)
  OPS_NOTIFY_MAIL_CMD      command fed the digest on stdin; unset = stdout
  OPS_NOTIFY_STATE_DIR     cursor + log location

Dormant (exit 0, silent) until the token file exists, so the cron can be
installed before the token is minted.
EOF
}

mode=poll
case "${1:-}" in
    --dry-run) mode=dry ;;
    --status) mode=status ;;
    --reset) mode=reset ;;
    -h | --help)
        usage
        exit 0
        ;;
    "") ;;
    *)
        echo "unknown option: $1 (see --help)" >&2
        exit 2
        ;;
esac

mkdir -p "$STATE_DIR"

log() {
    printf '%s %s\n' "$(date -Is)" "$*" >>"$LOG_FILE"
}

if [[ $mode == status ]]; then
    echo "url:    ${OPS_URL:-<unset — set OPS_BRAIN_URL>}"
    echo "agent:  $AGENT"
    if [[ -f $TOKEN_FILE ]]; then
        echo "token:  $TOKEN_FILE (mode $(stat -c '%a' "$TOKEN_FILE"))"
    else
        echo "token:  MISSING at $TOKEN_FILE — dormant"
    fi
    echo "mailer: ${MAIL_CMD:-<stdout>}"
    echo "cursor: $(cat "$CURSOR_FILE" 2>/dev/null || echo '<none>')"
    [[ -f $LOG_FILE ]] && { echo "--- recent ---"; tail -n 10 "$LOG_FILE"; }
    exit 0
fi

if [[ $mode == reset ]]; then
    rm -f "$CURSOR_FILE"
    echo "cursor cleared"
    exit 0
fi

# No deployment-specific default: a public reference implementation must not
# carry someone's host. Unlike a missing token this is a config error, not a
# not-yet-provisioned state, so it fails loudly.
if [[ -z $OPS_URL ]]; then
    echo "OPS_BRAIN_URL is not set — point it at your ops-brain deployment" >&2
    exit 2
fi

# Dormant until provisioned — a quiet exit keeps cron silent while the token
# mint is still pending.
if [[ ! -f $TOKEN_FILE ]]; then
    [[ $mode == dry ]] && echo "no token at $TOKEN_FILE — dormant" >&2
    exit 0
fi
perms=$(stat -c '%a' "$TOKEN_FILE")
if [[ $perms != 600 && $perms != 400 ]]; then
    log "REFUSING: $TOKEN_FILE has mode $perms (want 600)"
    echo "$TOKEN_FILE has mode $perms — chmod 600 it" >&2
    exit 1
fi
token=$(<"$TOKEN_FILE")
if [[ -z $token ]]; then
    log "REFUSING: token file empty"
    exit 1
fi

poll_started=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cursor=$(cat "$CURSOR_FILE" 2>/dev/null || true)
url="$OPS_URL/api/pending?agent=$AGENT"
[[ -n $cursor ]] && url="$url&since=$cursor"

if ! response=$(curl -sf --max-time 20 -H "Authorization: Bearer $token" "$url"); then
    log "poll failed: curl error against $OPS_URL"
    echo "operator-notify: poll failed against $OPS_URL" >&2
    exit 1
fi
if ! count=$(jq -r '.count' <<<"$response" 2>/dev/null); then
    log "poll failed: unparseable response"
    echo "operator-notify: unparseable response" >&2
    exit 1
fi

if ((count == 0)); then
    # Advance the cursor even on an empty poll: nothing new happened, and
    # holding the old cursor would re-report items on the next non-empty run.
    [[ $mode == poll ]] && echo "$poll_started" >"$CURSOR_FILE"
    exit 0
fi

# Body-free by design (the poll response carries no bodies). The digest gives
# enough to triage and a handoff id to point an agent at; full context is one
# `list_handoffs` away in a session.
digest=$(
    printf 'The team bus is blocked on you — %s new item(s).\n\n' "$count"
    jq -r '.items[] |
        "• [\(.priority)] \(.title)\n" +
        "    from \(.from_agent) · \(.status) · filed \(.created_at)" +
        (if .repeat_count > 0 then " · repeated \(.repeat_count)x" else "" end) +
        "\n    id \(.id)\n"' <<<"$response"
    printf '\nThese stay open until an agent completes them once unblocked.\n'
    printf 'Everything still open fleet-wide is in the daily briefing.\n'
)

if [[ $mode == dry ]]; then
    printf '%s\n' "$digest"
    echo "--- dry run: not sent, cursor not advanced (would be $poll_started) ---" >&2
    exit 0
fi

if [[ -n $MAIL_CMD ]]; then
    if ! printf '%s\n' "$digest" | eval "$MAIL_CMD"; then
        log "send FAILED via \$OPS_NOTIFY_MAIL_CMD — cursor NOT advanced"
        echo "operator-notify: send failed" >&2
        exit 1
    fi
else
    printf '%s\n' "$digest"
fi

echo "$poll_started" >"$CURSOR_FILE"
titles=$(jq -r '[.items[].title] | join("; ")' <<<"$response")
log "notified: $count item(s) — $titles"
exit 0
