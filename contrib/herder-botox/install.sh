#!/bin/sh
# Installs per-session reporting into the statusline script Claude Code already
# runs, plus the herdr sidebar/tab-row config. Idempotent; keeps a pristine copy
# of your original statusline for uninstall.sh, and a timestamped backup of
# whatever it replaces.
#
# Nothing here restarts herdr or your panes: the statusline is re-executed by
# Claude Code on every render, and `herdr server reload-config` applies the
# sidebar rows and tab-row entries live.
set -eu

ROOT=$(cd "$(dirname "$0")" && pwd)
HERDR_CONFIG=${HERDR_CONFIG_PATH:-$HOME/.config/herdr/config.toml}
CLAUDE_SETTINGS=$HOME/.claude/settings.json
TARGET=$HOME/.claude/statusline-command.sh
PRISTINE=$ROOT/config/statusline-original.sh
SNIPPET=$ROOT/config/herdr-snippet.toml
MARKER="# ── herder-botox ─"
MARKER_END="# ── /herder-botox ─"
KEYS_BEGIN="# ── herder-botox keys ─"
KEYS_END="# ── /herder-botox keys ─"
OURS="report_to_herdr"
STAMP=$(date +%Y%m%d-%H%M%S)

command -v jq >/dev/null 2>&1 || { echo "error: jq is required" >&2; exit 1; }
HERDR_BIN=$(command -v herdr 2>/dev/null) || { echo "error: herdr is not on PATH" >&2; exit 1; }
echo "herdr:      $HERDR_BIN ($("$HERDR_BIN" --version))"

# Soft check: prefix+y is spawned detached with no tty, so it cannot use
# herdr's own OSC 52 clipboard path and needs a native tool. Not fatal — the
# rest of the install does not depend on it.
CLIP=""
for tool in wl-copy xclip xsel pbcopy clip.exe; do
    command -v "$tool" >/dev/null 2>&1 && { CLIP=$tool; break; }
done
if [ -n "$CLIP" ]; then
    echo "clipboard:  $CLIP (prefix+y copies the tab name with this)"
else
    echo "clipboard:  WARNING none of wl-copy/xclip/xsel/pbcopy/clip.exe found." >&2
    echo "            prefix+y will have no way to reach a clipboard. Install one," >&2
    echo "            or switch that binding to type = \"popup\" — see the comment" >&2
    echo "            next to it in config/herdr-snippet.toml." >&2
fi

# ── 1. Confirm settings.json already points at the script we replace ────────
# We deliberately do not edit settings.json: it already runs this path, so no
# Claude Code session needs reloading.
if [ -f "$CLAUDE_SETTINGS" ]; then
    CONFIGURED=$(jq -r '.statusLine.command // ""' "$CLAUDE_SETTINGS")
else
    CONFIGURED=""
fi
case "$CONFIGURED" in
    *statusline-command.sh*) : ;;
    "")
        echo "error: no statusLine command is configured in $CLAUDE_SETTINGS." >&2
        echo "       Point it at $TARGET first, or install by hand." >&2
        exit 1 ;;
    *)
        echo "error: statusLine runs something else:" >&2
        echo "         $CONFIGURED" >&2
        echo "       This installer only extends $TARGET." >&2
        echo "       Merge bin/statusline-command.sh into your script by hand," >&2
        echo "       or repoint statusLine and rerun." >&2
        exit 1 ;;
esac

# ── 2. Preserve the original exactly once ───────────────────────────────────
[ -f "$TARGET" ] || { echo "error: $TARGET does not exist." >&2; exit 1; }
if grep -qF "$OURS" "$TARGET" 2>/dev/null; then
    echo "statusline: already extended, refreshing in place"
elif [ -f "$PRISTINE" ]; then
    echo "statusline: pristine copy already kept, not overwriting it"
else
    cp -p "$TARGET" "$PRISTINE"
    echo "pristine:   $PRISTINE"
fi
cp -p "$TARGET" "$TARGET.bak-$STAMP"
echo "backup:     $TARGET.bak-$STAMP"

# ── 3. Install, wiring the gauge library to an absolute path ────────────────
# Write-then-rename: a statusline render that lands mid-install reads either
# the old file or the new one, never a truncated one.
LIB=$ROOT/bin/ctx-bar.sh
[ -r "$LIB" ] || { echo "error: missing $LIB" >&2; exit 1; }

# herdr runs a tab-row command as `/bin/sh -lc "<command>"`, which execs the
# path and so needs the bit set. A checkout that lost it fails silently: the
# entry prints nothing, which is exactly what a working silent entry looks like.
chmod 755 "$ROOT"/bin/*.sh 2>/dev/null || true
ESCAPED=$(printf '%s' "$LIB" | sed 's/[&|\\]/\\&/g')
sed "s|__CTX_BAR_LIB__|$ESCAPED|" "$ROOT/bin/statusline-command.sh" >"$TARGET.new"
chmod --reference="$TARGET" "$TARGET.new" 2>/dev/null || chmod 755 "$TARGET.new"
mv "$TARGET.new" "$TARGET"
echo "statusline: $TARGET"

# ── 4. herdr config ────────────────────────────────────────────────────────
mkdir -p "$(dirname "$HERDR_CONFIG")"
touch "$HERDR_CONFIG"
cp -p "$HERDR_CONFIG" "$HERDR_CONFIG.bak-$STAMP"
echo "backup:     $HERDR_CONFIG.bak-$STAMP"

ROOT_ESCAPED=$(printf '%s' "$ROOT" | sed 's/[&|\\]/\\&/g')
expand_paths() { sed "s|\$HOME/herder-botox|$ROOT_ESCAPED|g"; }

# The snippet is one readable file; sentinels split it into the bare [ui] keys
# and the sub-tables, because TOML forbids declaring [ui] twice.
UI_KEYS=$(sed -n '/# >>> herder-botox ui-keys/,/# <<< herder-botox ui-keys/p' "$SNIPPET" \
    | sed '1d;$d')
UI_TABLES=$(sed -n '/# >>> herder-botox ui-tables/,$p' "$SNIPPET" | sed '1d' | expand_paths)

if grep -qF "$MARKER" "$HERDR_CONFIG" 2>/dev/null; then
    # Refresh in place instead of skipping, so a snippet change reaches an
    # existing install. Both ranges are sentinel-bounded, so anything herdr
    # wrote into [ui] itself sits outside them and is carried over untouched.
    # Hand edits INSIDE the markers are replaced — the backup above has them.
    tmp=$(mktemp)
    awk -v mk="$MARKER" -v mke="$MARKER_END" -v tables="$UI_TABLES" \
        -v kb="$KEYS_BEGIN" -v ke="$KEYS_END" -v keys="$UI_KEYS" '
        $0 == mk  { print mk; print tables; print mke; skip = 1; next }
        $0 == kb  { print kb; print keys;   print ke;  skip = 1; next }
        $0 == mke || $0 == ke { skip = 0; next }
        !skip { print }
    ' "$HERDR_CONFIG" >"$tmp"
    cat "$tmp" >"$HERDR_CONFIG"
    rm -f "$tmp"
    echo "herdr cfg:  block refreshed in place"
else
    # Refuse only on a genuine collision: a key or table we would define twice.
    CONFLICT=""
    for key in sidebar_width sidebar_min_width sidebar_max_width tab_bar_right_separator; do
        grep -qE "^[[:space:]]*$key[[:space:]]*=" "$HERDR_CONFIG" 2>/dev/null \
            && CONFLICT="$CONFLICT ui.$key"
    done
    for table in '[ui.sidebar.agents]' '[ui.sidebar.agents.rows_by_agent]' '[[ui.tab_bar_right]]'; do
        grep -qF "$table" "$HERDR_CONFIG" 2>/dev/null && CONFLICT="$CONFLICT $table"
    done
    # [[keys.command]] is deliberately absent from that list: it is an array of
    # tables, so entries from anywhere in the file merge. Only the key itself
    # can actually collide.
    grep -qE '^[[:space:]]*key[[:space:]]*=[[:space:]]*"prefix\+y"' "$HERDR_CONFIG" 2>/dev/null \
        && CONFLICT="$CONFLICT keys.command(prefix+y)"
    if [ -n "$CONFLICT" ]; then
        echo "error: $HERDR_CONFIG already defines:$CONFLICT" >&2
        echo "       Merge config/herdr-snippet.toml by hand instead." >&2
        exit 1
    fi

    # Our sub-tables go ABOVE the [ui] header, our bare keys just below it.
    # TOML allows a super-table after its sub-tables, and this placement is
    # what keeps [ui]'s bare-key zone running to EOF (or to whatever section
    # already followed it). herdr's own upsert appends a new [ui] key at the
    # end of that zone, so it never lands inside our block and uninstall.sh
    # never has to guess which keys were ours.
    tmp=$(mktemp)
    if grep -qE '^[[:space:]]*\[ui\][[:space:]]*$' "$HERDR_CONFIG"; then
        awk -v mk="$MARKER" -v mke="$MARKER_END" -v tables="$UI_TABLES" \
            -v kb="$KEYS_BEGIN" -v keys="$UI_KEYS" -v ke="$KEYS_END" '
            !seen && /^[[:space:]]*\[ui\][[:space:]]*$/ {
                print mk; print tables; print mke; print ""
                print; print kb; print keys; print ke
                seen = 1; next
            }
            { print }
        ' "$HERDR_CONFIG" >"$tmp"
        echo "herdr cfg:  merged into the existing [ui] section"
    else
        cp "$HERDR_CONFIG" "$tmp"
        printf '\n%s\n%s\n%s\n\n[ui]\n%s\n%s\n%s\n' \
            "$MARKER" "$UI_TABLES" "$MARKER_END" \
            "$KEYS_BEGIN" "$UI_KEYS" "$KEYS_END" >>"$tmp"
        echo "herdr cfg:  appended a new [ui] section"
    fi
    cat "$tmp" >"$HERDR_CONFIG"
    rm -f "$tmp"
fi

# ── 5. Apply to the running server, no restart ─────────────────────────────
if "$HERDR_BIN" server reload-config 2>&1 | sed 's/^/            /'; then
    echo "herdr:      config reloaded live"
else
    echo "herdr:      no running server to reload (applies on next start)"
fi

# ── 6. Seed state labels onto panes that already exist ────────────────────
# A session only reports when its statusline renders, and an idle one may not
# render for a while — so seed the labels now, or an already-idle agent shows
# no marker until you touch it. Same source as the reporter, so uninstall.sh
# clears these too.
# shellcheck source=bin/ctx-bar.sh
. "$LIB"
SEEDED=0
for pane in $("$HERDR_BIN" agent list 2>/dev/null | jq -r '.result.agents[]?.pane_id'); do
    "$HERDR_BIN" pane report-metadata "$pane" --source claude-ctx \
        --state-label "idle=$HERDR_LABEL_IDLE" \
        --state-label "done=$HERDR_LABEL_DONE" \
        --state-label "working=$HERDR_LABEL_WORKING" \
        --state-label "blocked=$HERDR_LABEL_BLOCKED" >/dev/null 2>&1 \
        && SEEDED=$(( SEEDED + 1 ))
done
[ "$SEEDED" -gt 0 ] && echo "labels:     seeded on $SEEDED existing pane(s)"

cat <<'DONE'

Done, and nothing was restarted.
  * settings.json untouched — it already runs that script
  * running agents keep running; each picks the gauge up on its next
    statusline render (a working agent renders continuously)
  * sidebar rows, tab-row entries and the prefix+y keybinding all came in
    through reload-config

prefix+y copies the current tab's name. The tab row confirms it for a moment,
since a keybinding herdr spawns detached has no other way to say anything.
Inside a session, bin/herdr-copy-tab.sh --print writes it to stdout instead.

Note: herdr remembers a manually dragged sidebar width in session.json, and
that beats `sidebar_width` from config. If the sidebar does not widen, drag its
right edge — the raised maximum applies immediately.

Revert with ./uninstall.sh
DONE
