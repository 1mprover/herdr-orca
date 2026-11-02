#!/bin/sh
# Shared gauge rendering. Sourced by the reporter and the aggregator.
#
# ctx_bar <percent> [cells]   -> "███████▊░░"  eighth-block resolution
# ctx_spark <percent>         -> "▇"           one cell, 8 height levels
#
# Style via HERDR_CTX_BAR_STYLE:
#   blocks (default)  █ ░ with partial ▏▎▍▌▋▊▉  — needs Block Elements coverage
#   ascii             # . — for fonts without it; remember to retune the
#                     starts_with rules in the herdr config if you switch

ctx_bar() {
    _pct=$1
    _cells=${2:-10}
    [ "$_pct" -lt 0 ] 2>/dev/null && _pct=0
    [ "$_pct" -gt 100 ] 2>/dev/null && _pct=100

    if [ "${HERDR_CTX_BAR_STYLE:-blocks}" = "ascii" ]; then
        _fill='#'; _empty='.'; _eighths=$(( (_pct * _cells + 50) / 100 * 8 ))
    else
        _fill='█'; _empty='░'; _eighths=$(( (_pct * _cells * 8 + 50) / 100 ))
    fi

    _full=$(( _eighths / 8 ))
    _rem=$(( _eighths % 8 ))
    [ "$_full" -gt "$_cells" ] && { _full=$_cells; _rem=0; }

    _out=''
    _i=0
    while [ "$_i" -lt "$_full" ]; do _out=$_out$_fill; _i=$(( _i + 1 )); done

    _partial=''
    case $_rem in
        1) _partial='▏' ;; 2) _partial='▎' ;; 3) _partial='▍' ;; 4) _partial='▌' ;;
        5) _partial='▋' ;; 6) _partial='▊' ;; 7) _partial='▉' ;;
    esac
    _out=$_out$_partial
    [ -n "$_partial" ] && _i=$(( _i + 1 ))

    while [ "$_i" -lt "$_cells" ]; do _out=$_out$_empty; _i=$(( _i + 1 )); done
    printf '%s' "$_out"
}

ctx_spark() {
    _pct=$1
    [ "$_pct" -lt 0 ] 2>/dev/null && _pct=0
    [ "$_pct" -gt 100 ] 2>/dev/null && _pct=100
    if [ "${HERDR_CTX_BAR_STYLE:-blocks}" = "ascii" ]; then
        if   [ "$_pct" -ge 75 ]; then printf 'H'
        elif [ "$_pct" -ge 50 ]; then printf 'M'
        elif [ "$_pct" -ge 25 ]; then printf 'l'
        else printf '.'
        fi
        return
    fi
    _lvl=$(( (_pct * 8 + 50) / 100 ))
    [ "$_lvl" -lt 1 ] && _lvl=1
    [ "$_lvl" -gt 8 ] && _lvl=8
    case $_lvl in
        1) printf '▁' ;; 2) printf '▂' ;; 3) printf '▃' ;; 4) printf '▄' ;;
        5) printf '▅' ;; 6) printf '▆' ;; 7) printf '▇' ;; *) printf '█' ;;
    esac
}

# ctx_band <percent> -> one of: calm warn high crit full
#
# herdr 0.8.2 has inline token styles (fg/bold/dim) but no conditional `rules`
# — those arrived after the release. So severity is chosen here and carried by
# WHICH token gets the gauge: the config gives each band its own fixed color,
# reporting fills exactly one and clears the rest, and herdr drops missing
# values along with their separators. Thresholds are therefore free of the
# gauge's cell count.
ctx_band() {
    _pct=$1
    if   [ "$_pct" -ge 90 ] 2>/dev/null; then printf 'full'
    elif [ "$_pct" -ge 80 ] 2>/dev/null; then printf 'crit'
    elif [ "$_pct" -ge 60 ] 2>/dev/null; then printf 'high'
    elif [ "$_pct" -ge 30 ] 2>/dev/null; then printf 'warn'
    else printf 'calm'
    fi
}

CTX_BANDS='calm warn high crit full'

# ctx_size <tokens> -> "303k" / "1.0M"
# 1M context windows render as "1000k" with a plain /1000, which reads badly.
ctx_size() {
    if [ "$1" -ge 1000000 ] 2>/dev/null; then
        printf '%d.%dM' "$(( $1 / 1000000 ))" "$(( $1 % 1000000 / 100000 ))"
    else
        printf '%dk' "$(( $1 / 1000 ))"
    fi
}

# Labels herdr shows for each agent state, overriding its plain
# idle/working/blocked/done. herdr picks the matching one itself as the state
# changes, so they are reported as static text and need no polling. herdr also
# colors them per state — green idle, teal done, yellow working, red blocked —
# so keep the calls to action loud and "working" quiet.
HERDR_LABEL_IDLE=${HERDR_LABEL_IDLE:-СВОБОДЕН}
HERDR_LABEL_DONE=${HERDR_LABEL_DONE:-ГОТОВО}
HERDR_LABEL_WORKING=${HERDR_LABEL_WORKING:-работает}
HERDR_LABEL_BLOCKED=${HERDR_LABEL_BLOCKED:-ЖДЁТ ОТВЕТА}

# Shown briefly in the tab row after bin/herdr-copy-tab.sh copies a tab name.
HERDR_LABEL_COPIED=${HERDR_LABEL_COPIED:-скопировано}
