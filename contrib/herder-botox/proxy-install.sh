#!/bin/sh
# Puts Claude sessions behind a local proxy relay whose upstream comes from
# ~/.zshrc, and binds prefix+u to switching it under running sessions.
# Idempotent; undo with ./proxy-uninstall.sh.
#
# Order matters: the relay is started and checked BEFORE anything points at
# it, so a relay — or an upstream — that cannot reach the API never takes the
# sessions down with it. Set HERDR_PROXY_FORCE=1 to point them at it anyway.
set -eu

ROOT=$(cd "$(dirname "$0")" && pwd)
PORT=${HERDR_PROXY_PORT:-18118}
RELAY_URL=http://127.0.0.1:$PORT
UNIT=herdr-proxy-relay.service
UNIT_FILE=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/$UNIT
HERDR_CONFIG=${HERDR_CONFIG_PATH:-$HOME/.config/herdr/config.toml}
CLAUDE_SETTINGS=$HOME/.claude/settings.json
SNIPPET=$ROOT/config/herdr-proxy-snippet.toml
MARKER="# ── herder-botox proxy ─"
MARKER_END="# ── /herder-botox proxy ─"
STATUS=${XDG_RUNTIME_DIR:-/tmp}/herdr-proxy/status
STAMP=$(date +%Y%m%d-%H%M%S)

for tool in jq herdr python3 zsh curl systemctl; do
    command -v "$tool" >/dev/null 2>&1 || { echo "error: $tool is required" >&2; exit 1; }
done
systemctl --user show-environment >/dev/null 2>&1 \
    || { echo "error: no systemd user manager to run the relay under" >&2; exit 1; }

# ── 1. What the relay will use ─────────────────────────────────────────────
# Host and port only: the URL may carry credentials.
UPSTREAM=$("$ROOT/bin/herdr-proxy-upstream.sh")
if [ -n "$UPSTREAM" ]; then
    echo "upstream:   $(printf '%s' "$UPSTREAM" | sed -E 's#^[A-Za-z0-9+.-]+://##; s#^[^@/]*@##; s#/.*$##') (from ~/.zshrc)"
else
    echo "upstream:   none in ~/.zshrc — the relay connects directly"
fi

# ── 2. Refuse to override a proxy settings.json already sets ───────────────
if [ -f "$CLAUDE_SETTINGS" ]; then
    jq empty "$CLAUDE_SETTINGS" 2>/dev/null \
        || { echo "error: $CLAUDE_SETTINGS is not valid JSON" >&2; exit 1; }
    FOREIGN=$(jq -r --arg relay "$RELAY_URL" '
        [(.env // {}) | to_entries[]
         | select((.key | ascii_downcase | test("^https?_proxy$")) and .value != $relay) | .key]
        | join(" ")' "$CLAUDE_SETTINGS")
    if [ -n "$FOREIGN" ]; then
        echo "error: $CLAUDE_SETTINGS already sets env: $FOREIGN" >&2
        echo "       Remove it first — the relay is meant to be the only proxy sessions see." >&2
        exit 1
    fi
fi

# ── 3. The relay, under systemd ───────────────────────────────────────────
chmod 755 "$ROOT"/bin/herdr-proxy-*.sh "$ROOT/bin/herdr-proxy-relay.py"
mkdir -p "$(dirname "$UNIT_FILE")"
RELAY_ESCAPED=$(printf '%s' "$ROOT/bin/herdr-proxy-relay.py" | sed 's/[&|\\]/\\&/g')
sed -e "s|__RELAY__|$RELAY_ESCAPED|" -e "s|__PORT__|$PORT|" \
    "$ROOT/config/herdr-proxy-relay.service" >"$UNIT_FILE"
systemctl --user daemon-reload
systemctl --user enable "$UNIT" >/dev/null 2>&1
systemctl --user restart "$UNIT"
echo "relay:      $UNIT on 127.0.0.1:$PORT"

# Wait out the relay's own check of the upstream (it gives up after 15s).
state=probing
i=0
while [ "$i" -lt 40 ]; do
    sleep 0.5
    i=$((i + 1))
    [ -r "$STATUS" ] && read -r _ _ _ state _ label error <"$STATUS" || continue
    [ "$state" = probing ] || break
done
case $state in
    ok) echo "check:      $label reaches the API" ;;
    *)
        echo "check:      ${label:-?} does NOT reach the API: ${error:-$state}" >&2
        if [ "${HERDR_PROXY_FORCE:-0}" != 1 ]; then
            echo "" >&2
            echo "Stopped before pointing any session at the relay: running sessions would" >&2
            echo "lose the connections they still have. Fix the proxy line in ~/.zshrc and" >&2
            echo "rerun, or rerun with HERDR_PROXY_FORCE=1. The relay keeps running; nothing" >&2
            echo "uses it yet. Log: journalctl --user -u $UNIT" >&2
            exit 1
        fi
        ;;
esac

# The relay's check proves the upstream; this proves the relay end to end.
if ! env -u HTTPS_PROXY -u HTTP_PROXY -u https_proxy -u http_proxy \
    curl -sS -o /dev/null --max-time 30 --proxy "$RELAY_URL" https://api.anthropic.com/ 2>/dev/null \
    && [ "${HERDR_PROXY_FORCE:-0}" != 1 ]; then
    echo "error: a request through $RELAY_URL failed; sessions were not pointed at it." >&2
    echo "       Log: journalctl --user -u $UNIT" >&2
    exit 1
fi

# ── 4. Point Claude sessions at the relay ─────────────────────────────────
# The env block of the user settings, not the shell: sessions read it at
# start, and Claude Code re-applies it, proxy agent included, when the file
# changes — so running sessions can move over without a restart.
# herdr-proxy-sessions.sh shows which ones did.
mkdir -p "$(dirname "$CLAUDE_SETTINGS")"
[ -f "$CLAUDE_SETTINGS" ] || echo '{}' >"$CLAUDE_SETTINGS"
cp -p "$CLAUDE_SETTINGS" "$CLAUDE_SETTINGS.bak-$STAMP"
echo "backup:     $CLAUDE_SETTINGS.bak-$STAMP"
jq --arg relay "$RELAY_URL" '.env = ((.env // {}) + {HTTPS_PROXY: $relay, HTTP_PROXY: $relay})' \
    "$CLAUDE_SETTINGS" >"$CLAUDE_SETTINGS.new"
chmod --reference="$CLAUDE_SETTINGS" "$CLAUDE_SETTINGS.new"
mv "$CLAUDE_SETTINGS.new" "$CLAUDE_SETTINGS"
echo "settings:   env.HTTPS_PROXY / env.HTTP_PROXY = $RELAY_URL"

# ── 5. herdr: prefix+u and the tab-row status ────────────────────────────
# Inserted right above the [ui] header — after the main herder-botox block
# when it is there — so that herdr's own writes into [ui] land below it and
# a refresh of this block never takes one of them along.
touch "$HERDR_CONFIG"
cp -p "$HERDR_CONFIG" "$HERDR_CONFIG.bak-$STAMP"
echo "backup:     $HERDR_CONFIG.bak-$STAMP"
ROOT_ESCAPED=$(printf '%s' "$ROOT" | sed 's/[&|\\]/\\&/g')
BLOCK=$(sed "s|\$HOME/herder-botox|$ROOT_ESCAPED|g" "$SNIPPET")
tmp=$(mktemp)
if grep -qF "$MARKER" "$HERDR_CONFIG"; then
    awk -v mk="$MARKER" -v mke="$MARKER_END" -v block="$BLOCK" '
        $0 == mk  { print mk; print block; print mke; skip = 1; next }
        $0 == mke { skip = 0; next }
        !skip { print }
    ' "$HERDR_CONFIG" >"$tmp"
    echo "herdr cfg:  proxy block refreshed in place"
elif grep -qE '^[[:space:]]*key[[:space:]]*=[[:space:]]*"prefix\+u"' "$HERDR_CONFIG"; then
    rm -f "$tmp"
    echo "error: $HERDR_CONFIG already binds prefix+u; merge $SNIPPET by hand." >&2
    exit 1
elif grep -qE '^[[:space:]]*\[ui\][[:space:]]*$' "$HERDR_CONFIG"; then
    awk -v mk="$MARKER" -v mke="$MARKER_END" -v block="$BLOCK" '
        !seen && /^[[:space:]]*\[ui\][[:space:]]*$/ { print mk; print block; print mke; print ""; seen = 1 }
        { print }
    ' "$HERDR_CONFIG" >"$tmp"
    echo "herdr cfg:  proxy block added above [ui]"
else
    { cat "$HERDR_CONFIG"; printf '\n%s\n%s\n%s\n' "$MARKER" "$BLOCK" "$MARKER_END"; } >"$tmp"
    echo "herdr cfg:  proxy block appended"
fi
if ! python3 -I -c 'import sys, tomllib; tomllib.load(open(sys.argv[1], "rb"))' "$tmp"; then
    rm -f "$tmp"
    echo "error: the merged herdr config is not valid TOML; $HERDR_CONFIG left as it was." >&2
    exit 1
fi
cat "$tmp" >"$HERDR_CONFIG"
rm -f "$tmp"
herdr config check | sed 's/^/            /'
if herdr server reload-config >/dev/null 2>&1; then
    echo "herdr:      config reloaded live"
else
    echo "herdr:      no running server to reload (applies on next start)"
fi

cat <<DONE

Done. Sessions now reach the network through $RELAY_URL, and only the relay
knows the upstream.

  prefix+u   re-read the proxy from ~/.zshrc; every session switches on its
             next request. The tab row confirms it for a few seconds and
             stays lit while the upstream fails.

Running sessions move over when Claude Code re-reads settings.json. Check
which ones did — an idle one shows up after its next request:

  $ROOT/bin/herdr-proxy-sessions.sh

A session listed as "мимо relay" still talks to the old upstream itself:
/exit it and start it again with claude --resume <id>.

Revert with ./proxy-uninstall.sh
DONE
