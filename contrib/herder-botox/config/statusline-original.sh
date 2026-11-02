#!/bin/sh
input=$(cat)
cwd=$(echo "$input" | jq -r '.workspace.project_dir // .cwd')
cwd=$(basename "$cwd")
model=$(echo "$input" | jq -r '.model.display_name')
used=$(echo "$input" | jq -r '.context_window.used_percentage // empty')
effort=$(echo "$input" | jq -r '.effort.level // empty')
five_h=$(echo "$input" | jq -r '.rate_limits.five_hour.used_percentage // empty')
week=$(echo "$input" | jq -r '.rate_limits.seven_day.used_percentage // empty')

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

printf "\033[38;5;135m%s\033[0m \033[38;5;208m[%s%s%s]\033[0m\033[38;5;220m%s\033[0m" \
    "$cwd" "$model" "$eff_info" "$ctx_info" "$limits_info"
