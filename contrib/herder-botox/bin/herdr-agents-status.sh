#!/bin/sh
# Fleet pulse for ui.tab_bar_right: a rotating glyph while anything is working,
# plus how many agents are blocked and how many are free and waiting for work.
#
# This is the only place a real animation is affordable. herdr does not animate
# its own state icons — the docs say the symbols are static — and Claude Code's
# OSC title glyph only advances when it happens to redraw its title, which is
# per-session and unreliable. A tab-row command entry, however, is polled by
# herdr on a fixed interval, so at interval_seconds = 1 the frame below turns
# once a second for one process spawn.
#
# Output example:  ◓ 3  × 1  ○ 2
#   ◐◓◑◒  rotates while at least one agent is working
#   ·     nothing is working
#   ×     blocked, needs an answer
#   ○     free (idle or done) — give it something to do
set -u

herdr_bin=${HERDR_BIN_PATH:-}
if [ -z "$herdr_bin" ]; then
    herdr_bin=$(command -v herdr 2>/dev/null) || exit 0
fi
[ -x "$herdr_bin" ] || exit 0
command -v jq >/dev/null 2>&1 || exit 0

counts=$("$herdr_bin" agent list 2>/dev/null | jq -r '
  [ .result.agents[]?.agent_status ] as $s
  | "\($s | map(select(. == "working")) | length) \($s | map(select(. == "blocked")) | length) \($s | map(select(. == "idle" or . == "done")) | length)"
') || exit 0
[ -n "$counts" ] || exit 0

# shellcheck disable=SC2086
set -- $counts
working=${1:-0}; blocked=${2:-0}; free=${3:-0}

# The frame comes from wall-clock seconds rather than stored state, so it keeps
# turning even though every poll is a fresh process.
if [ "$working" -gt 0 ]; then
    case $(( $(date +%s) % 4 )) in
        0) frame='◐' ;; 1) frame='◓' ;; 2) frame='◑' ;; *) frame='◒' ;;
    esac
else
    frame='·'
fi

out="$frame $working"
[ "$blocked" -gt 0 ] && out="$out  × $blocked"
[ "$free" -gt 0 ] && out="$out  ○ $free"
printf '%s\n' "$out"
