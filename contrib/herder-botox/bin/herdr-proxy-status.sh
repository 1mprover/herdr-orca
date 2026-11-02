#!/bin/sh
# Tab-row entry for the proxy relay. Silent while everything is fine — herdr
# drops an entry that prints nothing, separator included — and speaks up in
# three cases:
#
#   ⇄ kz.proxy.example.com ✓         a few seconds after prefix+u switched it,
#                                    or after a failing upstream recovered
#   ⇄ kz.proxy.example.com …         while the relay checks a new upstream
#   ⇄ ✗ kz.proxy.example.com: 407    while the upstream fails; the relay
#                                    re-checks every 30s and this clears
#   ⇄ relay не запущен               sessions point at a relay that is gone
#
# Reads one status line the relay writes, no jq: this runs every second.
set -u

WINDOW=${HERDR_PROXY_FLASH_SECONDS:-5}
case $WINDOW in ''|*[!0-9]*) WINDOW=5 ;; esac
LABEL_DOWN=${HERDR_LABEL_PROXY_DOWN:-relay не запущен}
LABEL_DIRECT=${HERDR_LABEL_PROXY_DIRECT:-без прокси}
STATUS=${XDG_RUNTIME_DIR:-/tmp}/herdr-proxy/status

pid=
[ -r "$STATUS" ] && read -r pid _ _ state since label error <"$STATUS"
# A pid alone could have been reused by an unrelated process.
if [ -z "$pid" ] || ! grep -qs herdr-proxy-relay "/proc/$pid/cmdline"; then
    printf '⇄ %s\n' "$LABEL_DOWN"
    exit 0
fi

host=${label%:*}
[ "$label" = direct ] && host=$LABEL_DIRECT

case $state in
    probing) printf '⇄ %s …\n' "$host" ;;
    error) printf '⇄ ✗ %s: %s\n' "$host" "$error" ;;
    ok)
        age=$(( $(date +%s) - since ))
        [ "$age" -ge 0 ] && [ "$age" -lt "$WINDOW" ] && printf '⇄ %s ✓\n' "$host"
        ;;
esac
exit 0
