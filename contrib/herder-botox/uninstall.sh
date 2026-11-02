#!/bin/sh
# Reverts install.sh: restores the original statusline script, strips the herdr
# config block, and clears reported tokens from every live pane.
set -eu

ROOT=$(cd "$(dirname "$0")" && pwd)
HERDR_CONFIG=${HERDR_CONFIG_PATH:-$HOME/.config/herdr/config.toml}
TARGET=$HOME/.claude/statusline-command.sh
PRISTINE=$ROOT/config/statusline-original.sh
MARKER="# ── herder-botox ─"
MARKER_END="# ── /herder-botox ─"
KEYS_BEGIN="# ── herder-botox keys ─"
KEYS_END="# ── /herder-botox keys ─"

HERDR_BIN=$(command -v herdr 2>/dev/null) || HERDR_BIN=""

# ── statusline ─────────────────────────────────────────────────────────────
if [ -f "$PRISTINE" ]; then
    cp -p "$PRISTINE" "$TARGET"
    echo "statusline: restored from $PRISTINE"
elif grep -qF "report_to_herdr" "$TARGET" 2>/dev/null; then
    echo "statusline: WARNING no pristine copy kept; $TARGET still extended." >&2
    echo "            Restore it from one of $TARGET.bak-* by hand." >&2
else
    echo "statusline: not extended, nothing to restore"
fi

# ── herdr config ───────────────────────────────────────────────────────────
# Two bounded ranges, nothing else. install.sh puts our sub-tables above the
# [ui] header and our bare keys just below it, which leaves [ui]'s bare-key
# zone ending outside our block — so anything herdr wrote there itself sits
# outside both ranges and survives untouched.
if [ -f "$HERDR_CONFIG" ] && grep -qF "$MARKER" "$HERDR_CONFIG"; then
    tmp=$(mktemp)
    awk -v mk="$MARKER" -v mke="$MARKER_END" -v kb="$KEYS_BEGIN" -v ke="$KEYS_END" '
        $0 == mk || $0 == kb { skip = 1; next }
        $0 == mke || $0 == ke { skip = 0; next }
        !skip { print }
    ' "$HERDR_CONFIG" >"$tmp"

    # Collapse the runs of blank lines the removal leaves behind.
    awk 'NF == 0 { blank = 1; next }
         { if (blank && NR > 1 && printed) print ""; blank = 0; printed = 1; print }
    ' "$tmp" >"$tmp.trim" && mv "$tmp.trim" "$tmp"
    cat "$tmp" >"$HERDR_CONFIG"
    rm -f "$tmp"
    echo "herdr cfg:  block removed"
fi

# ── clear reported metadata ────────────────────────────────────────────────
# Every token this ever wrote, including the ctxbar name used before the
# severity bands, plus the state labels — those are separate from tokens and
# need their own flag, or herdr keeps showing "СВОБОДЕН" after removal.
# tab1..tab3 come from another source (claude-tab), which does not matter for
# clearing: the pane's token store is one flat map and --source only tracks
# --seq freshness (src/metadata_tokens.rs:12).
OUR_TOKENS="ctxcalm ctxwarn ctxhigh ctxcrit ctxfull ctxbar ctx ctxb ctxt model eff tab1 tab2 tab3"

if [ -n "$HERDR_BIN" ] && command -v jq >/dev/null 2>&1; then
    CLEAR_ARGS=""
    for tok in $OUR_TOKENS; do
        CLEAR_ARGS="$CLEAR_ARGS --clear-token $tok"
    done

    # Select on any of our keys, not just one: a pane can be left holding only
    # a band token.
    "$HERDR_BIN" agent list 2>/dev/null \
        | jq -r --arg ours "$OUR_TOKENS" '
            ($ours | split(" ")) as $keys
            | .result.agents[]?
            | select(
                ((.tokens // {}) | keys | any(. as $k | $keys | index($k)))
                or ((.state_labels // {}) | length > 0)
              )
            | .pane_id
          ' \
        | while read -r pane; do
            # shellcheck disable=SC2086
            "$HERDR_BIN" pane report-metadata "$pane" --source claude-ctx \
                $CLEAR_ARGS --clear-state-labels >/dev/null 2>&1 || true
            echo "cleared:    $pane"
        done
    "$HERDR_BIN" server reload-config >/dev/null 2>&1 || true
fi

rm -rf "${XDG_RUNTIME_DIR:-/tmp}/herdr-ctx"
echo "Done. Timestamped .bak-* files are left in place."
