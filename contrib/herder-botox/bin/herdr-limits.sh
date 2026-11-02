#!/bin/sh
# Remaining subscription limits, for ui.tab_bar_right.
#
# Reads the snapshot that bin/statusline-command.sh writes. Rate limits are
# account-global, so any session's report is authoritative and the latest
# writer wins. Shows what is LEFT, not what is used.
#
# Output example:  5h 58% (2h14m) · 7d 88% (3d5h)
#
# The tab row strips ANSI (ESC is a control character), so this is monochrome
# by design. Stale snapshots are dropped rather than shown as current.
set -u

STALE_AFTER=${HERDR_LIMITS_STALE_AFTER:-3600}

state_file=${XDG_RUNTIME_DIR:-/tmp}/herdr-ctx/limits
[ -r "$state_file" ] || exit 0

five_used=''; five_reset=''; week_used=''; week_reset=''; written_at=''
while IFS='=' read -r key value; do
    case $key in
        5h_used)  five_used=$value ;;
        5h_reset) five_reset=$value ;;
        7d_used)  week_used=$value ;;
        7d_reset) week_reset=$value ;;
        at)       written_at=$value ;;
    esac
done <"$state_file"
[ -n "$written_at" ] || exit 0

now=$(date +%s)
# A snapshot older than an hour cannot be trusted to reflect the current
# window, and a wrong "you have plenty left" is worse than showing nothing.
[ $(( now - written_at )) -le "$STALE_AFTER" ] 2>/dev/null || exit 0

# Whole hours and minutes to a reset, in as few characters as reads clearly.
countdown() {
    _at=$1
    case $_at in ''|*[!0-9]*) return 0 ;; esac
    _left=$(( _at - now ))
    [ "$_left" -gt 0 ] || return 0
    _d=$(( _left / 86400 )); _h=$(( _left % 86400 / 3600 )); _m=$(( _left % 3600 / 60 ))
    if   [ "$_d" -gt 0 ]; then printf ' (%dd%dh)' "$_d" "$_h"
    elif [ "$_h" -gt 0 ]; then printf ' (%dh%02dm)' "$_h" "$_m"
    else printf ' (%dm)' "$_m"
    fi
}

# used_percentage -> remaining, rounded toward the pessimistic side so the
# number never overstates what is left.
remaining() {
    case $1 in ''|null) return 1 ;; esac
    _used=$(printf '%.0f' "$1" 2>/dev/null) || return 1
    _rem=$(( 100 - _used ))
    [ "$_rem" -lt 0 ] && _rem=0
    printf '%d' "$_rem"
}

out=''
if rem=$(remaining "${five_used:-}"); then
    out="5h $rem%$(countdown "${five_reset:-}")"
fi
if rem=$(remaining "${week_used:-}"); then
    [ -n "$out" ] && out="$out · "
    out="${out}7d $rem%$(countdown "${week_reset:-}")"
fi

[ -n "$out" ] && printf '%s\n' "$out"
