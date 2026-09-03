# ops-brain main-launcher integration for interactive POSIX shells.
#
# Source this file from ~/.bashrc or ~/.zshrc:
#
#   [ -r "$HOME/.local/share/ops-brain/shell-init.sh" ] && . "$HOME/.local/share/ops-brain/shell-init.sh"
#
# (the installer prints the exact line for its checkout or bundle). It defines
# `claude` and `codex` shell functions that route an attended terminal launch
# through the ops-brain launchers in --auto mode. Everything else — headless
# `claude -p`, `codex exec`, subcommands, pipes, scripts, systemd timers, wake
# shims — reaches the real binaries untouched, because shell functions are not
# inherited by child processes and --auto passes those shapes through anyway.
#
# The functions carry no credential. The launchers read the protected token
# file themselves; nothing here exports a bearer or touches the environment.
#
# Opt out for one launch:  claude --no-live      (or OPS_BRAIN_LIVE=off claude)
# Bypass the function:     command claude ...

# Interactive shells only. Under `set -u` an unset `$-` cannot happen, but the
# guard keeps `. shell-init.sh` harmless from a non-interactive script.
case "$-" in
    *i*) ;;
    *) return 0 ;;
esac

claude() {
    if command -v ops-brain-claude >/dev/null 2>&1; then
        ops-brain-claude --auto "$@"
    else
        command claude "$@"
    fi
}

codex() {
    if command -v ops-brain-codex >/dev/null 2>&1; then
        ops-brain-codex --auto "$@"
    else
        command codex "$@"
    fi
}
