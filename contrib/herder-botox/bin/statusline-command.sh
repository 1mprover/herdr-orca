#!/bin/sh
# Claude Code statusLine — your original script, plus per-session reporting
# into the herdr pane that hosts this session.
#
# The statusline output is byte-identical to the version this grew from
# (verified across 13 payload shapes, including the "[null]" model edge case).
# The additions are:
#   * one jq pass instead of seven — this runs on every render, and the
#     reporting needs four more fields
#   * a detached, throttled `herdr pane report-metadata` call
#   * a rate-limit snapshot dropped in the state dir for bin/herdr-limits.sh
#
# Reporting is entirely best-effort: no herdr, no pane, no gauge library, or a
# dead server all leave the statusline itself untouched.

input=$(cat)

# Single jq pass. The separator is US (0x1f), not a tab: tab counts as IFS
# whitespace, so `read` would collapse runs of it and shift every field after
# the first absent one. tostring reproduces `jq -r` exactly, including its
# literal "null" for an absent .model.display_name.
US=$(printf '\037')
fields=$(printf '%s' "$input" | jq -j --arg us "$US" '[
    ((.workspace.project_dir // .cwd) | tostring),
    (.model.display_name | tostring),
    ((.context_window.used_percentage // "") | tostring),
    ((.context_window.total_input_tokens // "") | tostring),
    ((.context_window.context_window_size // "") | tostring),
    ((.effort.level // "") | tostring),
    ((.rate_limits.five_hour.used_percentage // "") | tostring),
    ((.rate_limits.seven_day.used_percentage // "") | tostring),
    ((.rate_limits.five_hour.resets_at // "") | tostring),
    ((.rate_limits.seven_day.resets_at // "") | tostring)
] | join($us)')

IFS=$US read -r cwd model used used_tok win_size effort five_h week five_h_reset week_reset <<FIELDS
$fields
FIELDS

cwd=$(basename "$cwd")

esc=$(printf '\033')
if [ -n "$used" ]; then
    used_int=$(printf '%.0f' "$used")
    if [ "$used_int" -gt 30 ]; then
        ctx_info=" | ctx: ${esc}[38;5;160m${used_int}%${esc}[38;5;208m"
    else
        ctx_info=" | ctx: ${used_int}%"
    fi
else
    ctx_info=""
fi

if [ -n "$effort" ]; then
    eff_info=" | eff: $effort"
else
    eff_info=""
fi

lim_parts=""
if [ -n "$five_h" ]; then
    five_h_int=$(printf '%.0f' "$five_h")
    if [ "$five_h_int" -gt 85 ]; then
        lim_parts="5h: ${esc}[38;5;160m${five_h_int}%${esc}[38;5;220m"
    else
        lim_parts="5h: ${five_h_int}%"
    fi
fi
if [ -n "$week" ]; then
    [ -n "$lim_parts" ] && lim_parts="$lim_parts | "
    lim_parts="${lim_parts}7d: $(printf '%.0f' "$week")%"
fi

limits_info=""
[ -n "$lim_parts" ] && limits_info=" (Usage | $lim_parts)"

# ── herdr reporting ─────────────────────────────────────────────────────────
# Pane tokens rendered as $name in agent sidebar rows:
#   ctx<band>  "███████▊░░ 78%" gauge, written to exactly one of
#           ctxcalm / ctxwarn / ctxhigh / ctxcrit / ctxfull while the other
#           four are cleared. herdr 0.8.2 has no conditional token rules, so
#           the band picks the color by picking the token; the config gives
#           each one a fixed style and herdr hides the empty ones.
#   ctx     bare integer percent, for a plain numeric layout
#   ctxb    "156k/200k" breakdown
#   ctxt    raw used-token count, for the tab-row rollup
#   model   model display name
#   eff     effort level
#
# Rate limits are account-global, not per-session, so they are written once to
# a state file rather than repeated on every session row. Latest writer wins,
# which keeps the value fresh regardless of which session rendered last.
#
# install.sh rewrites the library path below to an absolute one; if it goes
# missing, reporting is skipped and the statusline still renders.
report_to_herdr() {
    [ -n "${HERDR_PANE_ID:-}" ] || return 0

    bar_lib=__CTX_BAR_LIB__
    [ -r "$bar_lib" ] || return 0

    herdr_bin=${HERDR_BIN_PATH:-}
    if [ -z "$herdr_bin" ]; then
        herdr_bin=$(command -v herdr 2>/dev/null) || return 0
    fi
    [ -n "$herdr_bin" ] && [ -x "$herdr_bin" ] || return 0

    state_dir=${XDG_RUNTIME_DIR:-/tmp}/herdr-ctx
    mkdir -p "$state_dir" 2>/dev/null || return 0
    now=$(date +%s)

    # Account-global rate limits: overwrite unconditionally, it is one cheap
    # write and staleness matters more here than churn.
    if [ -n "$five_h" ] || [ -n "$week" ]; then
        # key=value, not a separated list: an absent resets_at would leave an
        # empty field, and a space-separated reader collapses runs of those.
        printf '5h_used=%s\n5h_reset=%s\n7d_used=%s\n7d_reset=%s\nat=%s\n' \
            "${five_h:-}" "${five_h_reset:-}" "${week:-}" "${week_reset:-}" "$now" \
            >"$state_dir/limits.tmp.$$" 2>/dev/null \
            && mv "$state_dir/limits.tmp.$$" "$state_dir/limits" 2>/dev/null
    fi

    [ -n "${used_int:-}" ] || return 0

    # Throttle the socket roundtrip: the statusline re-renders several times a
    # second. Report on a percent change, or every 10s.
    state_file=$state_dir/$(printf '%s' "$HERDR_PANE_ID" | tr -c 'A-Za-z0-9_-' '_')
    if [ -r "$state_file" ]; then
        read -r last_pct last_at <"$state_file" 2>/dev/null || { last_pct=''; last_at=0; }
        if [ "$last_pct" = "$used_int" ] && [ $(( now - ${last_at:-0} )) -lt 10 ]; then
            return 0
        fi
    fi
    printf '%s %s\n' "$used_int" "$now" >"$state_file" 2>/dev/null

    # shellcheck source=bin/ctx-bar.sh
    . "$bar_lib" || return 0

    set -- --source claude-ctx \
        --state-label "idle=$HERDR_LABEL_IDLE" \
        --state-label "done=$HERDR_LABEL_DONE" \
        --state-label "working=$HERDR_LABEL_WORKING" \
        --state-label "blocked=$HERDR_LABEL_BLOCKED" \
        --token "ctx=$used_int"
    _band=$(ctx_band "$used_int")
    _gauge="$(ctx_bar "$used_int" "${HERDR_CTX_BAR_CELLS:-10}") $used_int%"
    for _b in $CTX_BANDS; do
        if [ "$_b" = "$_band" ]; then
            set -- "$@" --token "ctx$_b=$_gauge"
        else
            set -- "$@" --clear-token "ctx$_b"
        fi
    done
    if [ -n "${used_tok:-}" ] && [ -n "${win_size:-}" ] && [ "$win_size" -gt 0 ] 2>/dev/null; then
        set -- "$@" \
            --token "ctxb=$(ctx_size "$used_tok")/$(ctx_size "$win_size")" \
            --token "ctxt=$used_tok"
    fi
    [ -n "$model" ] && [ "$model" != "null" ] && set -- "$@" --token "model=$model"
    [ -n "$effort" ] && set -- "$@" --token "eff=$effort"

    # Detached so a slow or dead server never stalls the statusline.
    "$herdr_bin" pane report-metadata "$HERDR_PANE_ID" "$@" >/dev/null 2>&1 &
}

report_to_herdr 2>/dev/null

printf "\033[38;5;135m%s\033[0m \033[38;5;208m[%s%s%s]\033[0m\033[38;5;220m%s\033[0m" \
    "$cwd" "$model" "$eff_info" "$ctx_info" "$limits_info"
