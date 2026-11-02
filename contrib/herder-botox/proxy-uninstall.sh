#!/bin/sh
# Reverts proxy-install.sh. Sessions are pointed away from the relay first and
# the relay is stopped last, so a session that re-reads settings.json goes
# back to the HTTPS_PROXY it inherited from its shell before the relay is gone.
set -eu

PORT=${HERDR_PROXY_PORT:-18118}
RELAY_URL=http://127.0.0.1:$PORT
UNIT=herdr-proxy-relay.service
UNIT_FILE=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/$UNIT
HERDR_CONFIG=${HERDR_CONFIG_PATH:-$HOME/.config/herdr/config.toml}
CLAUDE_SETTINGS=$HOME/.claude/settings.json
MARKER="# ── herder-botox proxy ─"
MARKER_END="# ── /herder-botox proxy ─"
STAMP=$(date +%Y%m%d-%H%M%S)

# ── 1. settings.json: only the values that still name the relay ──────────
if [ -f "$CLAUDE_SETTINGS" ] && jq -e --arg relay "$RELAY_URL" \
    '(.env // {}) | (.HTTPS_PROXY == $relay or .HTTP_PROXY == $relay)' "$CLAUDE_SETTINGS" >/dev/null 2>&1; then
    cp -p "$CLAUDE_SETTINGS" "$CLAUDE_SETTINGS.bak-$STAMP"
    echo "backup:     $CLAUDE_SETTINGS.bak-$STAMP"
    jq --arg relay "$RELAY_URL" '
        .env |= with_entries(select(.value != $relay or (.key | test("^HTTPS?_PROXY$") | not)))
        | if .env == {} then del(.env) else . end' "$CLAUDE_SETTINGS" >"$CLAUDE_SETTINGS.new"
    chmod --reference="$CLAUDE_SETTINGS" "$CLAUDE_SETTINGS.new"
    mv "$CLAUDE_SETTINGS.new" "$CLAUDE_SETTINGS"
    echo "settings:   relay removed from env"
else
    echo "settings:   no relay in env, left as is"
fi

# ── 2. herdr config ───────────────────────────────────────────────────────
if grep -qF "$MARKER" "$HERDR_CONFIG" 2>/dev/null; then
    cp -p "$HERDR_CONFIG" "$HERDR_CONFIG.bak-$STAMP"
    echo "backup:     $HERDR_CONFIG.bak-$STAMP"
    tmp=$(mktemp)
    # The blank line the installer put after the block goes with it.
    awk -v mk="$MARKER" -v mke="$MARKER_END" '
        $0 == mk  { skip = 1; next }
        $0 == mke { skip = 0; after = 1; next }
        after && /^[[:space:]]*$/ { after = 0; next }
        { after = 0 }
        !skip { print }
    ' "$HERDR_CONFIG" >"$tmp"
    cat "$tmp" >"$HERDR_CONFIG"
    rm -f "$tmp"
    echo "herdr cfg:  proxy block removed"
    herdr server reload-config >/dev/null 2>&1 && echo "herdr:      config reloaded live" || true
fi

# ── 3. The relay itself ──────────────────────────────────────────────────
systemctl --user disable --now "$UNIT" >/dev/null 2>&1 || true
rm -f "$UNIT_FILE"
systemctl --user daemon-reload
echo "relay:      stopped and removed"

cat <<'DONE'

Done. A session that did not re-read settings.json still points at the relay,
which is gone now: /exit it and start it again with claude --resume <id>.
DONE
