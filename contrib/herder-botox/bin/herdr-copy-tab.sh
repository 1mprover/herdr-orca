#!/bin/sh
# Copy the name of the tab an agent session runs in.
#
# herdr copies text you SELECT inside a pane, but a tab name lives in the tab
# row and in the Agents panel — herdr's own chrome, not pane content, so there
# is nothing there to select. Nor is there a CLI for it: herdr's clipboard path
# runs pane OSC 52 -> server -> attached client (src/pane.rs:2183 ->
# src/client/mod.rs:1724), and only a pane can start it. So this script takes
# whichever route is actually open to it:
#
#   OSC 52  when it has a terminal — run inside a pane, or from a
#           type = "popup" keybinding. herdr forwards it to the terminal you
#           are attached with, which is the right clipboard even over
#           `herdr --remote`, and shows its own "copied" toast for free.
#   native  wl-copy / xclip / xsel / pbcopy / clip.exe otherwise. That is the
#           type = "shell" keybinding case: herdr spawns those detached with
#           stdin, stdout and stderr on /dev/null and no tty at all
#           (src/app/custom_commands.rs:288).
#
# Usage:
#   herdr-copy-tab.sh              copy this session's tab name
#   herdr-copy-tab.sh --print      print it, copy nothing
#   herdr-copy-tab.sh --id         copy the tab id (w1:tK) instead of the name
#   herdr-copy-tab.sh --tab  w1:tK target that tab
#   herdr-copy-tab.sh --pane w1:p3 target the tab holding that pane
#
# Env:
#   HERDR_COPY_BACKEND   auto (default) | osc52 | native
#   HERDR_COPY_FLASH     1 (default) | 0 — leave the stamp that
#                        herdr-copy-flash.sh turns into a tab-row confirmation
set -u

PROG=${0##*/}

usage() {
    cat <<USAGE
$PROG — copy the name of the tab an agent session runs in.

  $PROG                  copy this session's tab name
  $PROG --print          print it, copy nothing
  $PROG --id             copy the tab id (w1:tK) instead of the name
  $PROG --tab  w1:tK     target that tab
  $PROG --pane w1:p3     target the tab holding that pane
USAGE
}

tab_id=''
pane_id=''
want_id=0
print_only=0

while [ $# -gt 0 ]; do
    case $1 in
        -p|--print) print_only=1 ;;
        -i|--id)    want_id=1 ;;
        -t|--tab)   tab_id=${2:-};  [ $# -gt 1 ] && shift ;;
        --pane)     pane_id=${2:-}; [ $# -gt 1 ] && shift ;;
        -h|--help)  usage; exit 0 ;;
        *)
            printf '%s: unknown argument: %s\n' "$PROG" "$1" >&2
            usage >&2
            exit 2 ;;
    esac
    shift
done

herdr_bin=${HERDR_BIN_PATH:-}
if [ -z "$herdr_bin" ]; then
    herdr_bin=$(command -v herdr 2>/dev/null) || {
        printf '%s: herdr is not on PATH\n' "$PROG" >&2
        exit 1
    }
fi
[ -x "$herdr_bin" ] || { printf '%s: not executable: %s\n' "$PROG" "$herdr_bin" >&2; exit 1; }
command -v jq >/dev/null 2>&1 || { printf '%s: jq is required\n' "$PROG" >&2; exit 1; }

tab_of_pane() {
    "$herdr_bin" pane get "$1" 2>/dev/null | jq -r '.result.pane.tab_id // empty'
}

focused_tab() {
    "$herdr_bin" tab list 2>/dev/null \
        | jq -r '[.result.tabs[]? | select(.focused)][0].tab_id // empty'
}

if [ -n "$tab_id" ]; then
    :
elif [ -n "$pane_id" ]; then
    tab_id=$(tab_of_pane "$pane_id")
elif [ -n "${HERDR_ACTIVE_TAB_ID:-}" ]; then
    # Set only for [[keys.command]] processes: the tab that was focused when
    # the key was pressed. Checked before HERDR_PANE_ID because a popup command
    # runs in a pane of its own, and that pane's tab is not the one you meant.
    tab_id=$HERDR_ACTIVE_TAB_ID
elif [ -n "${HERDR_PANE_ID:-}" ]; then
    # Running inside a pane: the tab holding this very session.
    tab_id=$(tab_of_pane "$HERDR_PANE_ID")
else
    tab_id=$(focused_tab)
fi

[ -n "$tab_id" ] || { printf '%s: could not tell which tab to copy\n' "$PROG" >&2; exit 1; }

if [ "$want_id" -eq 1 ]; then
    text=$tab_id
else
    text=$("$herdr_bin" tab get "$tab_id" 2>/dev/null | jq -r '.result.tab.label // empty')
    [ -n "$text" ] || { printf '%s: no name for tab %s\n' "$PROG" "$tab_id" >&2; exit 1; }
fi

if [ "$print_only" -eq 1 ]; then
    printf '%s\n' "$text"
    exit 0
fi

# No trailing newline in any of these: pasting a tab name into a prompt should
# not also submit the line.
copy_osc52() {
    _b64=$(printf '%s' "$1" | base64 2>/dev/null | tr -d '\n') || return 1
    [ -n "$_b64" ] || return 1
    # "c" is the only target herdr accepts — its vt layer answers UNSUPPORTED
    # for any location other than the standard clipboard.
    if ( printf '\033]52;c;%s\a' "$_b64" >/dev/tty ) 2>/dev/null; then
        return 0
    fi
    if [ -t 1 ]; then
        printf '\033]52;c;%s\a' "$_b64"
        return 0
    fi
    return 1
}

copy_native() {
    if command -v wl-copy >/dev/null 2>&1; then
        printf '%s' "$1" | wl-copy >/dev/null 2>&1 && return 0
    fi
    if command -v xclip >/dev/null 2>&1; then
        printf '%s' "$1" | xclip -selection clipboard -in >/dev/null 2>&1 && return 0
    fi
    if command -v xsel >/dev/null 2>&1; then
        printf '%s' "$1" | xsel --clipboard --input >/dev/null 2>&1 && return 0
    fi
    if command -v pbcopy >/dev/null 2>&1; then
        printf '%s' "$1" | pbcopy >/dev/null 2>&1 && return 0
    fi
    if command -v clip.exe >/dev/null 2>&1; then
        printf '%s' "$1" | clip.exe >/dev/null 2>&1 && return 0
    fi
    return 1
}

# A type = "shell" keybinding has no tty, so it cannot reach herdr's own
# "copied" toast either. The stamp is what bin/herdr-copy-flash.sh turns into a
# short-lived tab-row confirmation, which is the one channel left.
flash_stamp() {
    [ "${HERDR_COPY_FLASH:-1}" = 1 ] || return 0
    _dir=${XDG_RUNTIME_DIR:-/tmp}/herdr-ctx
    mkdir -p "$_dir" 2>/dev/null || return 0
    printf '%s\n' "$(date +%s)" >"$_dir/copied.tmp.$$" 2>/dev/null \
        && mv "$_dir/copied.tmp.$$" "$_dir/copied" 2>/dev/null
    rm -f "$_dir/copied.tmp.$$" 2>/dev/null
    return 0
}

copied=0
case ${HERDR_COPY_BACKEND:-auto} in
    osc52)   copy_osc52 "$text" && copied=1 ;;
    native)  copy_native "$text" && copied=1 ;;
    auto|'') copy_osc52 "$text" && copied=1 || { copy_native "$text" && copied=1; } ;;
    *)
        printf '%s: unknown HERDR_COPY_BACKEND: %s\n' "$PROG" "$HERDR_COPY_BACKEND" >&2
        exit 2 ;;
esac

if [ "$copied" -eq 1 ]; then
    flash_stamp
    # Only when someone is watching: keeps `--print`-less use pipe-clean.
    [ -t 1 ] && printf '%s\n' "$text"
    exit 0
fi

printf '%s: no way to reach a clipboard from here (no tty for OSC 52, and no\n' "$PROG" >&2
printf '       wl-copy / xclip / xsel / pbcopy / clip.exe on PATH)\n' >&2
printf '%s\n' "$text"
exit 1
