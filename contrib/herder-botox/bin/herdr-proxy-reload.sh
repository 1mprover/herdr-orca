#!/bin/sh
# prefix+u: re-read the upstream proxy from ~/.zshrc and move every session
# onto it.
#
# Sessions talk to the local relay (herdr-proxy-relay.py), never to the
# upstream, so nothing inside them has to change: a reload makes the relay
# re-read ~/.zshrc and drop its open tunnels, and each session's next request
# goes out through the new upstream. A relay that is not running is started
# instead, which reads ~/.zshrc anyway.
#
# herdr spawns this detached and silent, so the tab row reports the switch
# (herdr-proxy-status.sh). Run from a terminal, it waits for the relay's check
# of the new upstream and prints the outcome.
set -u

UNIT=herdr-proxy-relay.service
STATUS=${XDG_RUNTIME_DIR:-/tmp}/herdr-proxy/status

generation=
[ -r "$STATUS" ] && read -r _ generation _ <"$STATUS"

if ! systemctl --user reload-or-restart "$UNIT" 2>/dev/null; then
    echo "herdr-proxy: could not reload $UNIT — see: journalctl --user -u $UNIT" >&2
    exit 1
fi

[ -t 1 ] || exit 0

# The probe gives up after 15s, so this cannot outwait it by much.
i=0
while [ "$i" -lt 40 ]; do
    sleep 0.5
    i=$((i + 1))
    [ -r "$STATUS" ] || continue
    read -r _ now _ state _ label error <"$STATUS" || continue
    [ "$now" != "$generation" ] && [ "$state" != probing ] || continue
    echo "upstream: $label — $state${error:+ ($error)}"
    exit 0
done
echo "upstream: no answer from the relay's check yet — see: journalctl --user -u $UNIT" >&2
exit 1
