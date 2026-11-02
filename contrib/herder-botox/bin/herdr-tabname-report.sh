#!/bin/sh
# Wraps each agent session's tab name onto as many sidebar rows as it needs and
# reports the pieces as $tab1 / $tab2 / $tab3.
#
# Why this exists: the built-in `tab` token is one line and herdr truncates it
# with an ellipsis at the row's budget (`truncate_end`, src/ui/text.rs:7); a row
# is never wrapped. At a 36-column sidebar a continuation row has 31 columns —
# 35 of content, minus the 3-column indent herdr applies to every row after the
# first (src/client/shell/agent_sidebar.rs:357), minus the scrollbar column — so
# a name like BRIEF-checklist-participants-click-2026-09-02 cannot fit on one
# row under any layout. Splitting it across rows is the only way to show all of
# it without widening the sidebar.
#
# Why a poller and not the statusline reporter: this has to stay correct for a
# session that is idle and rendering nothing, and it has to notice a tab rename.
# herdr polls ui.tab_bar_right entries on its own clock and drops a status entry
# whose command prints nothing (src/server/client_shell.rs:189), so this rides
# that timer and prints nothing at all. One process per interval for the whole
# fleet, and it only writes when a wrapped name actually changed.
#
# Env:
#   HERDR_TABNAME_WIDTH   wrap width; default is the live sidebar width minus 5
#                         (1 divider + 3 indent + 1 scrollbar)
#   HERDR_TABNAME_LINES   1..3, how many rows the name may use (default 3)
set -u

herdr_bin=${HERDR_BIN_PATH:-}
if [ -z "$herdr_bin" ]; then
    herdr_bin=$(command -v herdr 2>/dev/null) || exit 0
fi
[ -x "$herdr_bin" ] || exit 0
command -v jq >/dev/null 2>&1 || exit 0

CONFIG=${HERDR_CONFIG_PATH:-$HOME/.config/herdr/config.toml}
STATE=$(dirname "$CONFIG")/session.json

# Same precedence herdr itself uses: a width dragged by hand is persisted as a
# preference and beats config, because the effective width only follows config
# while sidebar_width_manual is false (src/client/shell/config.rs:69).
width=${HERDR_TABNAME_WIDTH:-}
if [ -z "$width" ]; then
    width=$(jq -r '.sidebar_width // empty' "$STATE" 2>/dev/null)
    [ -n "$width" ] || width=$(sed -n 's/^[[:space:]]*sidebar_width[[:space:]]*=[[:space:]]*\([0-9]\{1,\}\).*/\1/p' "$CONFIG" 2>/dev/null | head -1)
    [ -n "$width" ] || width=26
    width=$(( width - 5 ))
fi
case $width in ''|*[!0-9]*) width=21 ;; esac
[ "$width" -ge 8 ] || width=8

lines=${HERDR_TABNAME_LINES:-3}
case $lines in 1|2|3) : ;; *) lines=3 ;; esac

agents=$("$herdr_bin" agent list 2>/dev/null) || exit 0
tabs=$("$herdr_bin" tab list 2>/dev/null) || exit 0

# Codepoint-safe wrapping: jq slices strings by codepoint, so a Cyrillic tab
# name cannot be cut mid-character the way a byte-based cut would. Breaks are
# preferred at - _ . / or space and the separator stays at the end of the line,
# which makes a split slug read as deliberate; a break in the first half of the
# row is ignored in favour of a hard cut, so one early hyphen cannot waste the
# whole row.
rows=$(printf '%s\n%s\n' "$agents" "$tabs" | jq -s -r --argjson w "$width" '
  def brk: [45, 95, 46, 47, 32];
  def cut($t; $w):
    ($t | explode) as $c
    | if ($c | length) <= $w then [$t, ""]
      else
        ([range(0; $w) | select($c[.] as $ch | brk | index($ch) != null)] | last) as $p
        | if ($p == null) or ((($p + 1) * 2) < $w)
          then [$t[0:$w], $t[$w:]]
          else [$t[0:($p + 1)], $t[($p + 1):]]
          end
      end;
  def wrap3($t; $w):
    cut($t; $w) as [$a, $r1]
    | if ($r1 | length) == 0 then [$a, "", ""]
      else cut($r1; $w) as [$b, $r2]
        | if ($r2 | length) == 0 then [$a, $b, ""] else [$a, $b, $r2] end
      end;
  (.[0].result.agents // []) as $agents
  | (.[1].result.tabs // []) as $tabs
  | ($tabs | group_by(.workspace_id)
           | map({key: .[0].workspace_id, value: length}) | from_entries) as $counts
  | $agents[]
  | . as $a
  | ([$tabs[] | select(.tab_id == $a.tab_id)] | first) as $t
  | if $t == null then empty
    else
      # herdr hides its own tab token for a lone auto-named tab; match that
      # rather than dedicating rows to a name that is only the tab number.
      (($counts[$t.workspace_id] // 1) == 1 and ($t.label == ($t.number | tostring))) as $auto
      | (if $auto then ["", "", ""] else wrap3($t.label; $w) end) as $parts
      | [$a.pane_id] + $parts | join("\u001f")
    end
') || exit 0
[ -n "$rows" ] || exit 0

state_dir=${XDG_RUNTIME_DIR:-/tmp}/herdr-ctx
mkdir -p "$state_dir" 2>/dev/null || exit 0

US=$(printf '\037')
while IFS=$US read -r pane l1 l2 l3; do
    [ -n "${pane:-}" ] || continue
    [ "$lines" -ge 3 ] || l3=''
    [ "$lines" -ge 2 ] || l2=''

    cache=$state_dir/tabname_$(printf '%s' "$pane" | tr ':/' '__')
    current="$width$US$l1$US$l2$US$l3"
    if [ -r "$cache" ]; then
        read -r previous <"$cache" 2>/dev/null || previous=''
        [ "$previous" = "$current" ] && continue
    fi

    set -- --source claude-tab
    for n in 1 2 3; do
        eval "value=\$l$n"
        if [ -n "$value" ]; then
            set -- "$@" --token "tab$n=$value"
        else
            set -- "$@" --clear-token "tab$n"
        fi
    done
    if "$herdr_bin" pane report-metadata "$pane" "$@" >/dev/null 2>&1; then
        printf '%s\n' "$current" >"$cache.tmp.$$" 2>/dev/null \
            && mv "$cache.tmp.$$" "$cache" 2>/dev/null
        rm -f "$cache.tmp.$$" 2>/dev/null
    fi
done <<ROWS
$rows
ROWS
exit 0
