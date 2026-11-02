#!/bin/sh
# Fleet rollup for ui.tab_bar_right: one sparkline cell per live agent session,
# ordered by workspace/pane so the cells keep a stable position, plus the peak.
#
# Reads the ctx/ctxt tokens that herdr-ctx-report.sh puts on each pane.
# Note: the tab row strips ANSI (ESC is a control character), so this output is
# monochrome by design. Color lives in the sidebar rows instead.
#
# Output example:  ctx ▂▇▄ 197k max 78%
set -u

# shellcheck source=bin/ctx-bar.sh
. "$(dirname "$0")/ctx-bar.sh"

herdr_bin=${HERDR_BIN_PATH:-}
if [ -z "$herdr_bin" ]; then
    herdr_bin=$(command -v herdr 2>/dev/null) || exit 0
fi
[ -x "$herdr_bin" ] || exit 0
command -v jq >/dev/null 2>&1 || exit 0

# "<pct> <used_tokens>" per reporting session, stably ordered.
rows=$("$herdr_bin" agent list 2>/dev/null | jq -r '
  [ .result.agents[]? | select(.tokens.ctx != null) ]
  | sort_by(.workspace_id, .pane_id)
  | .[] | "\(.tokens.ctx) \(.tokens.ctxt // "0")"
') || exit 0
[ -n "$rows" ] || exit 0

spark=''
total=0
peak=0
count=0
while read -r pct used; do
    [ -n "$pct" ] || continue
    spark=$spark$(ctx_spark "$pct")
    total=$(( total + used ))
    [ "$pct" -gt "$peak" ] && peak=$pct
    count=$(( count + 1 ))
done <<ROWS
$rows
ROWS

[ "$count" -gt 0 ] || exit 0
printf 'ctx %s %dk max %d%%\n' "$spark" "$(( total / 1000 ))" "$peak"
