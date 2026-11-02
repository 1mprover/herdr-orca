#!/bin/sh
# Prints the upstream proxy URL that ~/.zshrc exports, or nothing when it
# exports none (the relay then connects directly).
#
# Only the proxy export lines are evaluated, in a bare `zsh -f`: sourcing the
# whole rc would start oh-my-zsh and everything else in it for one variable.
# Commented-out alternatives are skipped, so switching stays what it was —
# comment one line out, uncomment another, press prefix+u. A line that builds
# the value from another one, like HTTP_PROXY="$HTTPS_PROXY", still resolves.
#
# Precedence is Claude Code's own: https_proxy, HTTPS_PROXY, http_proxy,
# HTTP_PROXY. Override the file with HERDR_PROXY_RC.
set -eu

RC=${HERDR_PROXY_RC:-$HOME/.zshrc}
[ -r "$RC" ] || exit 0

grep -E '^[[:space:]]*export[[:space:]]+(https?_proxy|HTTPS?_PROXY)=' "$RC" | zsh -f -c '
    unset https_proxy HTTPS_PROXY http_proxy HTTP_PROXY
    eval "$(cat)"
    print -r -- "${https_proxy:-${HTTPS_PROXY:-${http_proxy:-${HTTP_PROXY:-}}}}"
'
