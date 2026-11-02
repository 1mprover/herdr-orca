#!/bin/sh
# Transient copy confirmation for ui.tab_bar_right.
#
# herdr spawns a type = "shell" keybinding detached, with no tty, so
# herdr-copy-tab.sh cannot reach herdr's own "copied" toast from there and
# prefix+y would otherwise be completely silent. This prints a marker for a few
# seconds after a copy and nothing the rest of the time: herdr drops a status
# entry whose command prints nothing, its separator included
# (src/server/client_shell.rs:189), so the segment stays invisible until it has
# something to say.
#
# Output:  ⎘ скопировано   (for HERDR_COPY_FLASH_SECONDS after a copy)
set -u

# shellcheck source=bin/ctx-bar.sh
. "$(dirname "$0")/ctx-bar.sh"

WINDOW=${HERDR_COPY_FLASH_SECONDS:-3}
case $WINDOW in ''|*[!0-9]*) WINDOW=3 ;; esac

stamp_file=${XDG_RUNTIME_DIR:-/tmp}/herdr-ctx/copied
[ -r "$stamp_file" ] || exit 0

read -r stamped <"$stamp_file" 2>/dev/null || exit 0
case $stamped in ''|*[!0-9]*) exit 0 ;; esac

age=$(( $(date +%s) - stamped ))
[ "$age" -ge 0 ] && [ "$age" -lt "$WINDOW" ] || exit 0

printf '⎘ %s\n' "$HERDR_LABEL_COPIED"
