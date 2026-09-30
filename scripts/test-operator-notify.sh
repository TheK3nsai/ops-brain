#!/usr/bin/env bash
#
# test-operator-notify.sh — fixture tests for operator-notify.sh.
#
# operator-notify.sh is the only push path that reaches the human, and it runs
# unattended from cron, so its failure contract is the thing under test: a
# failed poll or send never advances the cursor, every real run reports to the
# dead-man monitor, --dry-run touches nothing, and no secret reaches argv or
# --status output.
#
# curl is stubbed on PATH and records every call; jq is the real one. Run from
# anywhere: scripts/test-operator-notify.sh

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
NOTIFY="$SCRIPT_DIR/operator-notify.sh"

TOKEN='fixture-machine-token-0123456789abcdef'
PUSH_SECRET='fixture-push-secret'
BEAT_URL="https://kuma.example.invalid/api/push/$PUSH_SECRET"

pass_count=0
tmp_root=$(mktemp -d)
trap 'rm -rf "$tmp_root"' EXIT

fail() {
    printf 'not ok - %s\n' "$*" >&2
    exit 1
}

pass() {
    pass_count=$((pass_count + 1))
    printf 'ok %d - %s\n' "$pass_count" "$*"
}

assert_contains() {
    [[ $1 == *"$2"* ]] || fail "$3: expected [$2] in [$1]"
}

assert_not_contains() {
    [[ $1 != *"$2"* ]] || fail "$3: did not expect [$2] in [$1]"
}

# Fresh fixture per case: token, state dir, and a curl stub. The stub answers
# /api/pending from $fx/response (exit status from $fx/poll-exit) and any other
# URL as a heartbeat (exit status from $fx/beat-exit). It logs its argv, the
# contents of any --config it was handed, and every heartbeat URL it saw,
# whether that URL arrived in argv or through --config.
new_fixture() {
    fx=$(mktemp -d "$tmp_root/case.XXXXXX")
    mkdir -p "$fx/bin" "$fx/state"
    printf '%s\n' "$TOKEN" >"$fx/token"
    chmod 600 "$fx/token"
    printf '{"count":0,"items":[]}\n' >"$fx/response"
    for log in curl-argv curl-config beats; do : >"$fx/$log"; done

    cat >"$fx/bin/curl" <<EOF
#!/usr/bin/env bash
fx='$fx'
printf '%s\n' "\$*" >>"\$fx/curl-argv"
urls=()
while [[ \$# -gt 0 ]]; do
    case \$1 in
        --config | -K)
            config=\$(<"\$2")
            printf '%s\n' "\$config" >>"\$fx/curl-config"
            while IFS= read -r line; do
                [[ \$line =~ ^url[[:space:]]*=[[:space:]]*\"(.*)\"\$ ]] && urls+=("\${BASH_REMATCH[1]}")
            done <<<"\$config"
            shift 2
            ;;
        --max-time | -H | --header) shift 2 ;;
        http*) urls+=("\$1"); shift ;;
        *) shift ;;
    esac
done
for url in "\${urls[@]}"; do
    if [[ \$url == *"/api/pending"* ]]; then
        printf '%s\n' "\$url" >>"\$fx/polls"
        code=\$(cat "\$fx/poll-exit" 2>/dev/null || echo 0)
        [[ \$code == 0 ]] && cat "\$fx/response"
        exit "\$code"
    fi
    printf '%s\n' "\$url" >>"\$fx/beats"
    exit "\$(cat "\$fx/beat-exit" 2>/dev/null || echo 0)"
done
exit 2
EOF
    chmod +x "$fx/bin/curl"
}

# run_notify [args...] — sets $out, $err, $status. Extra environment comes
# from the caller's exported variables.
run_notify() {
    local default_mail="cat >'$fx/mail'"
    status=0
    out=$(
        PATH="$fx/bin:$PATH" \
            HOME="$fx/home" \
            OPS_BRAIN_URL="${FX_URL-https://ops.example.invalid}" \
            OPS_NOTIFY_TOKEN_FILE="$fx/token" \
            OPS_NOTIFY_STATE_DIR="$fx/state" \
            OPS_NOTIFY_HEARTBEAT_URL="$BEAT_URL" \
            OPS_NOTIFY_MAIL_CMD="${FX_MAIL-$default_mail}" \
            "$NOTIFY" "$@" 2>"$fx/stderr"
    ) || status=$?
    err=$(<"$fx/stderr")
}

cursor() { cat "$fx/state/since" 2>/dev/null || true; }
beats() { cat "$fx/beats"; }

two_items='{"count":2,"items":[
 {"id":"01a0aaaa-0000-7000-8000-000000000001","priority":"high","title":"Blocked: approve the restore window","from_agent":"Claude-Example","status":"pending","created_at":"2026-09-29T12:00:00Z","repeat_count":0},
 {"id":"01a0aaaa-0000-7000-8000-000000000002","priority":"medium","title":"Blocked: renew the certificate","from_agent":"Codex-Example","status":"accepted","created_at":"2026-09-28T08:00:00Z","repeat_count":3}]}'

test_dormant_without_token() {
    new_fixture
    rm "$fx/token"
    run_notify
    [[ $status -eq 0 && -z $out ]] || fail "dormant run should be silent and exit 0 (status=$status out=[$out])"
    [[ ! -s $fx/curl-argv ]] || fail "dormant run must not call curl"
    pass "no token file: dormant, silent, no network"
}

test_missing_url_is_loud() {
    new_fixture
    FX_URL='' run_notify
    [[ $status -eq 2 ]] || fail "missing OPS_BRAIN_URL should exit 2 (got $status)"
    assert_contains "$(beats)" "status=down" "missing URL reports down"
    [[ ! -e $fx/polls ]] || fail "missing URL must not poll"
    pass "unset OPS_BRAIN_URL fails loudly and reports down"
}

test_refuses_loose_or_empty_token() {
    new_fixture
    chmod 644 "$fx/token"
    run_notify
    [[ $status -eq 1 ]] || fail "mode 644 token should exit 1 (got $status)"
    assert_contains "$(beats)" "status=down" "loose token reports down"
    [[ ! -e $fx/polls ]] || fail "loose token must not be used to poll"
    assert_contains "$(cat "$fx/state/notify.log")" "REFUSING" "loose token logged"

    new_fixture
    : >"$fx/token"
    run_notify
    [[ $status -eq 1 ]] || fail "empty token should exit 1 (got $status)"
    assert_contains "$(beats)" "status=down" "empty token reports down"

    # Would be mangled in, or break out of, the quoted curl config line.
    new_fixture
    printf 'fixture-token-with-"quote"\n' >"$fx/token"
    run_notify
    [[ $status -eq 1 ]] || fail "a token with a quote should exit 1 (got $status)"
    assert_contains "$(beats)" "status=down&msg=token%20file%20malformed" "malformed token reports down"
    [[ ! -e $fx/polls ]] || fail "a malformed token must not be used to poll"
    pass "group-readable, empty or malformed token is refused before any poll"
}

test_empty_queue_advances_cursor() {
    new_fixture
    run_notify
    [[ $status -eq 0 ]] || fail "empty poll should exit 0 (got $status: $err)"
    [[ $(cursor) =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:]{8}Z$ ]] || fail "empty poll should set the cursor (got [$(cursor)])"
    [[ ! -e $fx/mail ]] || fail "empty poll must not send"
    assert_contains "$(beats)" "status=up" "empty poll reports up"
    assert_contains "$(cat "$fx/polls")" "/api/pending?agent=Operator" "polls the operator slug"
    pass "empty queue: nothing sent, cursor advanced, heartbeat up"
}

test_items_are_sent_and_cursor_used() {
    local first_cursor
    new_fixture
    printf '%s\n' "$two_items" >"$fx/response"
    run_notify
    [[ $status -eq 0 ]] || fail "send should exit 0 (got $status: $err)"
    local mail
    mail=$(<"$fx/mail")
    assert_contains "$mail" "blocked on you — 2 new item(s)" "digest header"
    assert_contains "$mail" "• [high] Blocked: approve the restore window" "digest item line"
    assert_contains "$mail" "from Codex-Example · accepted · filed 2026-09-28T08:00:00Z · repeated 3x" "digest repeat count"
    assert_contains "$mail" "id 01a0aaaa-0000-7000-8000-000000000001" "digest carries the handoff id"
    assert_not_contains "$mail" "from Claude-Example · pending · filed 2026-09-29T12:00:00Z · repeated" "no repeat note at zero"
    assert_contains "$(beats)" "status=up&msg=notified%202%20item" "heartbeat up with an encoded message"
    first_cursor=$(cursor)
    [[ -n $first_cursor ]] || fail "a successful send should set the cursor"

    printf '{"count":0,"items":[]}\n' >"$fx/response"
    run_notify
    assert_contains "$(tail -n1 "$fx/polls")" "&since=$first_cursor" "next poll resumes from the cursor"
    pass "items: digest mailed, heartbeat up, next poll resumes from the cursor"
}

test_send_failure_holds_cursor() {
    new_fixture
    printf '%s\n' "$two_items" >"$fx/response"
    printf '2026-01-01T00:00:00Z\n' >"$fx/state/since"
    FX_MAIL="cat >'$fx/mail' && false" run_notify
    [[ $status -eq 1 ]] || fail "failed send should exit 1 (got $status)"
    [[ $(cursor) == 2026-01-01T00:00:00Z ]] || fail "failed send must not advance the cursor (got $(cursor))"
    assert_contains "$(beats)" "status=down&msg=send%20failed" "failed send reports down"
    assert_contains "$(cat "$fx/state/notify.log")" "send FAILED" "failed send logged"
    pass "send failure: exit 1, cursor held for retry, heartbeat down"
}

test_poll_failure_holds_cursor() {
    new_fixture
    printf '2026-01-01T00:00:00Z\n' >"$fx/state/since"
    printf '22\n' >"$fx/poll-exit"
    run_notify
    [[ $status -eq 1 ]] || fail "failed poll should exit 1 (got $status)"
    [[ $(cursor) == 2026-01-01T00:00:00Z ]] || fail "failed poll must not advance the cursor"
    assert_contains "$(beats)" "status=down&msg=poll%20failed" "failed poll reports down"
    [[ ! -e $fx/mail ]] || fail "failed poll must not send"
    pass "poll failure: exit 1, cursor held, heartbeat down"
}

test_bad_response_holds_cursor() {
    local body
    # Not JSON, and JSON that is not a pending response (an error envelope, or
    # a count that is not a number). None of them may read as an empty queue.
    for body in '<html>bad gateway</html>' '{"error":"upstream"}' '{"count":"2","items":[]}' \
        '{"count":1.5,"items":[]}' '{"count":-1,"items":[]}'; do
        new_fixture
        printf '2026-01-01T00:00:00Z\n' >"$fx/state/since"
        printf '%s\n' "$body" >"$fx/response"
        run_notify
        [[ $status -eq 1 ]] || fail "response [$body] should exit 1 (got $status)"
        [[ $(cursor) == 2026-01-01T00:00:00Z ]] || fail "response [$body] must not advance the cursor"
        assert_contains "$(beats)" "status=down" "response [$body] reports down"
        assert_not_contains "$(beats)" "status=up" "response [$body] never reports up"
    done
    pass "a response that is not a pending count fails, holds the cursor, reports down"
}

test_heartbeat_failure_is_not_fatal() {
    new_fixture
    printf '%s\n' "$two_items" >"$fx/response"
    printf '7\n' >"$fx/beat-exit"
    run_notify
    [[ $status -eq 0 ]] || fail "an unreachable monitor must not fail the run (got $status)"
    [[ -n $(cursor) && -s $fx/mail ]] || fail "an unreachable monitor must not block the send"
    assert_contains "$(cat "$fx/state/notify.log")" "heartbeat ping failed" "monitor failure logged"
    pass "unreachable monitor: logged, never changes exit code, send or cursor"
}

test_dry_run_touches_nothing() {
    new_fixture
    printf '%s\n' "$two_items" >"$fx/response"
    printf '2026-01-01T00:00:00Z\n' >"$fx/state/since"
    run_notify --dry-run
    [[ $status -eq 0 ]] || fail "dry run should exit 0 (got $status)"
    assert_contains "$out" "2 new item(s)" "dry run prints the digest"
    [[ ! -e $fx/mail ]] || fail "dry run must not send"
    [[ $(cursor) == 2026-01-01T00:00:00Z ]] || fail "dry run must not advance the cursor"
    [[ ! -s $fx/beats ]] || fail "dry run must not ping the monitor"

    # The empty-queue and failure paths are where a real run advances the
    # cursor or reports, so they are where a dry run has to hold back.
    new_fixture
    printf '2026-01-01T00:00:00Z\n' >"$fx/state/since"
    run_notify --dry-run
    [[ $status -eq 0 ]] || fail "dry run on an empty queue should exit 0 (got $status)"
    [[ $(cursor) == 2026-01-01T00:00:00Z ]] || fail "dry run on an empty queue must not advance the cursor"
    [[ ! -s $fx/beats ]] || fail "dry run on an empty queue must not report up"

    new_fixture
    printf '22\n' >"$fx/poll-exit"
    run_notify --dry-run
    [[ $status -eq 1 ]] || fail "dry run with a failed poll should still exit 1 (got $status)"
    [[ ! -s $fx/beats ]] || fail "dry run with a failed poll must not report down"
    pass "--dry-run: prints only, no send, no cursor, no heartbeat on any path"
}

test_stdout_without_mailer() {
    new_fixture
    printf '%s\n' "$two_items" >"$fx/response"
    FX_MAIL='' run_notify
    [[ $status -eq 0 ]] || fail "stdout delivery should exit 0 (got $status)"
    assert_contains "$out" "renew the certificate" "no mailer: digest on stdout"
    [[ -n $(cursor) ]] || fail "stdout delivery should advance the cursor"
    pass "no mailer: digest goes to stdout and counts as sent"
}

test_secrets_stay_out_of_argv_and_status() {
    new_fixture
    printf '%s\n' "$two_items" >"$fx/response"
    run_notify
    [[ $status -eq 0 ]] || fail "run should succeed (got $status: $err)"
    assert_not_contains "$(cat "$fx/curl-argv")" "$TOKEN" "machine token kept out of curl argv"
    assert_not_contains "$(cat "$fx/curl-argv")" "$PUSH_SECRET" "push token kept out of curl argv"
    # The exact config line, quoting included: unquoted, real curl would cut
    # the header at the first space and send a bearer-less request.
    grep -Fqx "header = \"Authorization: Bearer $TOKEN\"" "$fx/curl-config" ||
        fail "bearer delivered through --config as a quoted header line (got [$(cat "$fx/curl-config")])"

    run_notify --status
    assert_not_contains "$out" "$PUSH_SECRET" "--status elides the push token"
    assert_not_contains "$out" "$TOKEN" "--status never prints the machine token"
    assert_contains "$out" "https://kuma.example.invalid/api/push/…" "--status shows the elided monitor"
    pass "machine and push tokens never reach argv or --status"
}

test_reset_clears_cursor() {
    new_fixture
    printf '2026-01-01T00:00:00Z\n' >"$fx/state/since"
    run_notify --reset
    [[ $status -eq 0 && -z $(cursor) ]] || fail "--reset should clear the cursor"
    pass "--reset clears the cursor"
}

test_dormant_without_token
test_missing_url_is_loud
test_refuses_loose_or_empty_token
test_empty_queue_advances_cursor
test_items_are_sent_and_cursor_used
test_send_failure_holds_cursor
test_poll_failure_holds_cursor
test_bad_response_holds_cursor
test_heartbeat_failure_is_not_fatal
test_dry_run_touches_nothing
test_stdout_without_mailer
test_secrets_stay_out_of_argv_and_status
test_reset_clears_cursor

printf '1..%d\n' "$pass_count"
