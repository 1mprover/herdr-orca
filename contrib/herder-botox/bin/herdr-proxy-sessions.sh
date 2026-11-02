#!/bin/sh
# Which Claude sessions already go through the proxy relay, judged by the
# connections each claude process has open right now.
#
#   relay 2            everything goes through the relay; prefix+u reaches it
#   мимо relay 3       talks to an upstream directly — restart this session
#                      (/exit, then claude --resume <id> in a fresh shell)
#   relay 1, мимо 2    moving over: new requests use the relay, the old
#                      keep-alive connections die off on their own
#   нет соединений     idle with nothing open; its next request decides
#
# "Directly" means a connection to any port that a proxy line in ~/.zshrc
# names, commented-out alternatives included.
set -u

PORT=${HERDR_PROXY_PORT:-18118}
RC=${HERDR_PROXY_RC:-$HOME/.zshrc}
TAB=$(printf '\t')

PROXY_PORTS=$(grep -E '^[[:space:]#]*export[[:space:]]+(https?_proxy|HTTPS?_PROXY)=' "$RC" 2>/dev/null \
    | sed -nE 's#.*://([^/"@]*@)?[^/":]+:([0-9]+).*#\2#p' | sort -u | tr '\n' ' ')
CONNECTIONS=$(ss -tnpH state established 2>/dev/null)

herdr pane list 2>/dev/null | jq -r '
    .result.panes[] | select(.agent == "claude")
    | [.pane_id, ([.tokens.tab1, .tokens.tab2, .tokens.tab3] | map(. // "") | join("")),
       .terminal_title_stripped] | @tsv' |
while IFS=$TAB read -r pane tab title; do
    pid=$(herdr pane process-info --pane "$pane" 2>/dev/null | jq -r '
        [.result.process_info.foreground_processes[]? | select(.name == "claude")][0].pid // empty')
    if [ -z "$pid" ]; then
        verdict="claude не найден"
    else
        counts=$(printf '%s\n' "$CONNECTIONS" | grep -F "pid=$pid," | awk -v relay="127.0.0.1:$PORT" \
            -v ports=" $PROXY_PORTS" '
            { peer = $4; n = split(peer, parts, ":"); port = parts[n] }
            peer == relay { r++; next }
            peer !~ /^(127\.|\[::1\]|\[::ffff:127\.)/ && index(ports, " " port " ") { d++ }
            END { printf "%d %d", r, d }')
        relay=${counts% *}
        direct=${counts#* }
        if [ "$relay" -gt 0 ] && [ "$direct" -gt 0 ]; then
            verdict="relay $relay, мимо $direct"
        elif [ "$relay" -gt 0 ]; then
            verdict="relay $relay"
        elif [ "$direct" -gt 0 ]; then
            verdict="мимо relay $direct"
        else
            verdict="нет соединений"
        fi
    fi
    printf '%-7s %-20s %s\n' "$pane" "$verdict" "${tab:-$title}"
done
