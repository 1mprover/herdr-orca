# herder-botox

Per-session model, effort and context gauge in herdr's Agents panel, plus a
context rollup and remaining subscription limits in the tab row, and `prefix+y`
to copy the name of the tab a session runs in.

Built on top of your existing `~/.claude/statusline-command.sh` — its critical
context threshold and its palette carry over, and its own output is unchanged.

Verified against the **installed herdr 0.8.2 binary** and Claude Code
**2.1.263**. That distinction matters: herdr's git HEAD is ahead of the 0.8.2
release, and two things this originally relied on — the `machine` sidebar token
and conditional token `rules` — exist only there. Both were caught by the
running server and worked around; see *Coloring the gauge*.

## The short version

Nothing here needs a herdr fork. Every request so far has landed on an
extension point that already exists:

| Idea | Status |
|---|---|
| Widen the Agents panel | `ui.sidebar_max_width` has no upper ceiling in herdr — the default 36 is just a default. Config only. |
| Per-session context, model, effort | Custom `$token` sidebar values already exist, fed by `herdr pane report-metadata`. Needs a reporter, not a patch. |
| Rollup, limits, fleet pulse | `ui.tab_bar_right` runs shell commands on an interval, 1s minimum. Config + three scripts. |
| Green "free, give me work" marker | Native: herdr already colors idle green. Needs `status_indicators` + a `--state-label`, no reporter logic. |
| Copy the tab name | `[[keys.command]]` binds a key to a shell command and hands it `HERDR_ACTIVE_TAB_ID`. Config + one script. |
| Show a long tab name in full | herdr never wraps a row, so the name is wrapped in a reporter and arrives as one token per line. Config + one script. |
| Switch the proxy under running sessions | herdr has no custom buttons or menu items, but it forwards a click to any pane app that turns on mouse reporting. A tab with one pane is the button: it restarts the sessions still on another proxy with `claude --resume`. See *Switching the proxy*; a no-restart relay is there as well. |

What *does* need upstream work is narrower than the original idea, and it is
written up with file:line references in [`upstream/NOTES.md`](upstream/NOTES.md):
the Agents panel cannot exceed 90% of sidebar **height** and that split is
mouse-only, and there is no *native* context token — so today every agent needs
its own reporter.

## What you get

In the Agents panel, per Claude session:

```
◐ Opus 5 (1M context) · xhigh
  работает · ███▌░░░░░░ 35%
  BRIEF-checklist-copy-new-
  template-2026-09-03
  modules.git

○ Fable 5.1 · xhigh
  СВОБОДЕН · ██▎░░░░░░░ 22%
  BRIEF-bizproc-trusted-files-
  2026-09-02
  modules.git
```

The tab name is shown **in full**, wrapped across as many rows as it needs.
That is not cosmetic: the built-in `tab` token is one line and herdr truncates
it at the row's budget (`truncate_end`, `src/ui/text.rs:7`), and a 36-column
sidebar leaves 31 columns on a continuation row, so a name like
`BRIEF-checklist-participants-click-2026-09-02` cannot fit on one row under any
layout. `bin/herdr-tabname-report.sh` splits it and reports the pieces as
`$tab1` / `$tab2` / `$tab3`; the rows it does not need disappear.

The state line is loud on purpose: green **СВОБОДЕН** means that agent is
waiting for work, teal ГОТОВО means it finished something you have not looked
at, red ЖДЁТ ОТВЕТА means it is blocked on a question, and lowercase yellow
работает stays quiet because it needs nothing from you. Rename them with
`HERDR_LABEL_IDLE` and friends in `bin/ctx-bar.sh`.

The gauge has eighth-block resolution, so it moves on small changes instead of
stepping once per 10%. Its escalation starts at **30%** — the critical point
taken from your statusline script, where it flips ctx to red — and then keeps
climbing rather than staying flat:

```
  0% -> dim          ░░░░░░░░░░
 30% -> #ffd700      ███░░░░░░░
 60% -> #ff8700      ██████░░░░
 80% -> #d70000      ████████░░
 90% -> #d70000 bold █████████░
```

Those four colors are your own xterm indices 220 / 208 / 160 converted to hex,
so the panel matches the statusline it grew from.

In the tab row — a turning glyph while anything works, a sparkline cell per
session, and what is **left** of your limits:

```
◓ 2  ○ 2 │ ctx ▂▂▃ 868k max 35% │ 5h 57% (2h00m) · 7d 93% (5d17h)
```

Three independent entries, so you can reorder or drop any of them. The limits
part shows remaining, not used — your statusline shows used, and 42% used is
the same reading as 58% left. `◓ 2  ○ 2` is two agents working and two free;
a `× 1` appears between them when something is blocked.

And **`prefix+y` copies the current tab's name** — the generated task slug a
session runs under, like `BRIEF-checklist-copy-new-template-2026-09-03`, ready
to paste into a commit message or a ticket. A fourth tab-row entry confirms it
for a few seconds and is invisible the rest of the time:

```
◓ 2  ○ 2 │ ctx ▂▂▃ 868k max 35% │ 5h 57% (2h00m) · 7d 93% (5d17h) │ ⎘ скопировано
```

From inside a session the same script prints it instead, which is handy when
you want the name in a command rather than the clipboard:

```sh
bin/herdr-copy-tab.sh --print          # this session's tab name
bin/herdr-copy-tab.sh --id             # the tab id, w1:tK
bin/herdr-copy-tab.sh --tab w1:tK      # some other tab
```

## About the spinner

The turning glyph lives in the tab row rather than on each session row, and
that placement is a finding rather than a preference.

herdr does not animate its own state icons — `status_indicators` is documented
as static, and `status_icon` (`src/client/shell.rs:194`) is a plain match from
state to one fixed glyph. Claude Code *does* push a rotating `◐◓◑◒` into its
OSC title, which herdr keeps in the `terminal_title` token, so that looked like
free animation. Sampling four live panes every 0.6s for eight seconds killed
the idea: one pane alternated, the other three sat on a single frame the whole
time. The glyph only advances when Claude Code happens to redraw its title,
which is per-session and not a clock — selling it as a spinner would have been
wrong.

Animating a per-session token instead would mean a `report-metadata` call per
frame per agent — several process spawns and socket roundtrips per second — for
decoration. A tab-row command entry, by contrast, is polled by herdr on a fixed
interval, and 1 second is the documented minimum. So `bin/herdr-agents-status.sh`
derives its frame from wall-clock seconds (`date +%s % 4`), which keeps turning
even though every poll is a fresh process. Measured cost through the login shell
herdr uses: 0.01s.

Per-session, motion comes from the things that actually change: the state label
flips as herdr reclassifies the agent, and the gauge creeps as context fills.

## About the green

This part turned out to be native. herdr already colors agent state
(`status_color`, `src/client/shell.rs:241`): working yellow, blocked red, done
teal, **idle green**, unknown grey. Two config choices surface it:

- `status_indicators = "symbols"` gives each state its own shape — `×` blocked,
  `◐` working, `✓` done, `○` idle. Under the default `dots`, everything except
  idle is the same `●` and only color separates them.
- `state_text` is drawn in that same per-state color, but `DIM` unless the row
  is focused. The row overrides `bold = true, dim = false` and deliberately
  sets no `fg`, since an explicit color would replace the semantic one with a
  single fixed value.

The text itself comes from `--state-label`, which herdr stores per pane and
switches on its own as the state changes — no polling, and it keeps working
while a session sits idle and renders nothing.

## About copying the tab name

herdr copies text you select with the mouse, and even a double-clicked token,
but a tab name is not selectable text: it lives in the tab row and in the
Agents panel, which are herdr's own chrome, drawn from state rather than from
pane content. There is no CLI for it either. herdr's clipboard plumbing does
exist and is complete — a pane emits OSC 52, the server turns it into
`AppEvent::ClipboardWrite` (`src/pane.rs:2183`), the attached client re-emits
it to the outer terminal and pops its own "copied" toast
(`src/client/mod.rs:1724`) — but the only way in is *from a pane*. `herdr pane
send-text` and `pane run` write to a pane's **input**, which for an agent pane
means typing into the agent's prompt, so they are not it.

So `bin/herdr-copy-tab.sh` takes whichever route is open to it:

| Route | When | Reaches |
|---|---|---|
| OSC 52 | it has a tty — run inside a pane, or from a `type = "popup"` binding | the terminal you are *attached with*, correct even over `herdr --remote`, plus herdr's built-in toast |
| native | no tty | `wl-copy` / `xclip` / `xsel` / `pbcopy` / `clip.exe` on the machine herdr runs on |

The `prefix+y` binding is `type = "shell"`, which herdr spawns detached with
stdin, stdout and stderr all on `/dev/null`
(`src/app/custom_commands.rs:288`) — no tty, so the OSC 52 path is out of reach
there and it uses `xclip`. That also means it cannot say anything: no output
goes anywhere, and herdr's own toast belongs to the clipboard path it did not
use. Hence the fourth tab-row entry. `bin/herdr-copy-flash.sh` prints a marker
for three seconds after a copy and nothing otherwise, which works because herdr
drops a status entry whose command prints nothing — separator included
(`src/server/client_shell.rs:189`), so the segment is genuinely invisible
between copies rather than an empty gap.

The alternative is in the snippet, commented: the same script on a `popup`
binding gets a real pane, hence a tty, hence OSC 52, herdr's native toast and
the right clipboard when you are attached from another machine. It costs a
brief popup flash on every copy. Both can coexist on separate keys.

Which tab it copies is settled by `HERDR_ACTIVE_TAB_ID`, which herdr sets for
custom commands (along with `HERDR_ACTIVE_WORKSPACE_ID`, `_PANE_ID` and
`_PANE_CWD`) — all four confirmed present in the installed 0.8.2 binary, since
they are recent enough to be worth checking. That variable is preferred over
`HERDR_PANE_ID` precisely because a popup runs in a pane of its own, whose tab
is not the one you meant. Run from inside a session with no keybinding
involved, `HERDR_PANE_ID` is what resolves the tab; failing both, the focused
tab from `herdr tab list`.

Measured end to end, replicating the spawn herdr performs — server environment,
`/bin/sh -lc`, all three descriptors on `/dev/null`, new session — the copy
completes in **0.034s**.

## How it works

`install.sh` extends `~/.claude/statusline-command.sh` in place. **`settings.json`
is not touched** — it already runs that path. Claude Code hands the script a
JSON payload containing an exact `context_window` object (`used_percentage`,
`total_input_tokens`, `context_window_size` — built in `Ilo()` inside the Claude
binary), so nothing is scraped or estimated.

The extended script reports these tokens onto the pane named by `$HERDR_PANE_ID`:

- `ctxcalm` / `ctxwarn` / `ctxhigh` / `ctxcrit` / `ctxfull` — the gauge
  `███████▊░░ 78%` lands in exactly one of these, by severity; the other four
  are cleared on every report
- `ctx` — bare integer percent, for a plain numeric layout
- `ctxb` — `306k/1.0M` breakdown, switching to `M` above 999k so a 1M window
  does not read as `1000k`
- `ctxt` — raw used-token count, for the rollup
- `model` — model display name
- `eff` — effort level

plus the four `--state-label` overrides, which are not tokens and are stored
separately. `install.sh` seeds those onto every existing pane as its last step,
because a session only reports when its statusline renders and an already-idle
one may not render for a while — without seeding, an idle agent would show no
marker until you touched it.

**Wrapping the tab name** is a second reporter, `bin/herdr-tabname-report.sh`,
and it does not run from the statusline. It has to stay right for a session
that is idle and rendering nothing, and it has to notice a tab rename, so it
rides herdr's own tab-row timer instead: a `ui.tab_bar_right` entry that prints
nothing, which herdr drops whole, entry and separator
(`src/server/client_shell.rs:189`). One process every 5 seconds for the entire
fleet — measured at 0.06s — and it writes to a pane only when that pane's
wrapped name actually changed, which in the steady state is never.

The wrap width follows the sidebar: the persisted width from `session.json`
first, then `sidebar_width` from config, then herdr's own default of 26, minus
5 for the divider, the 3-column indent herdr puts on every row after the first
(`src/client/shell/agent_sidebar.rs:357`) and the scrollbar column. That
precedence is herdr's own — a width you dragged by hand is stored as a
preference and beats config, because the effective width only follows config
while `sidebar_width_manual` is false (`src/client/shell/config.rs:69`).
Override it with `HERDR_TABNAME_WIDTH`, or cap the row count with
`HERDR_TABNAME_LINES=1|2|3`.

Breaks are preferred at `-`, `_`, `.`, `/` or a space, with the separator kept
at the end of the line so a split slug reads as deliberate rather than clipped;
a break in the first half of a row is passed over in favour of a hard cut, so
one early hyphen cannot waste a row. The slicing runs in `jq`, which indexes
strings by codepoint, so a Cyrillic tab name cannot be cut mid-character the
way a byte-based cut would.

Unused lines are **cleared**, not reported empty. That is the same mechanism
the gauge bands use, and it matters here: a row whose every token is missing is
dropped entirely (`(!resolved.is_empty()).then_some(resolved)`,
`src/ui/sidebar/tokens.rs:119`), but an empty-*but-present* token keeps its row
and renders a blank line.

The copy marker uses the same state dir: `bin/herdr-copy-tab.sh` writes a
timestamp there and `bin/herdr-copy-flash.sh` renders the marker while that
timestamp is younger than `HERDR_COPY_FLASH_SECONDS` (3). Set
`HERDR_COPY_FLASH=0` to stop stamping, or `HERDR_COPY_BACKEND=osc52|native` to
pin the route instead of letting it pick.

Rate limits are account-global rather than per-session, so they are written
once to a snapshot in the state dir instead of being repeated on every row.
Latest writer wins, which keeps the value fresh no matter which session
rendered last, and `bin/herdr-limits.sh` reads it back for the tab row.

**Your statusline output is byte-identical** to the script this grew from,
checked across 13 payload shapes including absent context, absent effort, one
limit window only, an empty object, and the `[null]` model case. Two things
changed inside it: seven `jq` invocations became one (it runs on every render,
and the reporting needs four more fields), and the reporting call itself, which
is throttled and detached so a slow or dead herdr server cannot stall your
prompt. If the gauge library goes missing, reporting is skipped and the
statusline still renders.

**Coloring the gauge** is where the released binary forced a redesign. herdr
0.8.2 supports inline token styles (`fg`, `bold`, `dim`) but not the
conditional `rules` that make a style depend on the value — those landed after
the release, confirmed by grepping the installed binary for the four error
strings that code emits (zero hits each). A gauge string is not a number
anyway, so numeric thresholds were never going to reach it.

The workaround uses a documented 0.8.2 behaviour instead: *missing values and
their separators disappear*. Severity is decided in the shell, and the band
picks its color by picking its token — each report writes the gauge into one of
five differently styled tokens and clears the other four, so exactly one ever
renders. Thresholds are plain percentages, which also frees them from the
gauge's cell count:

```
  <30   ctxcalm   dim
  30+   ctxwarn   #ffd700
  60+   ctxhigh   #ff8700
  80+   ctxcrit   #d70000
  90+   ctxfull   #d70000 bold
```

A commented numeric layout in the snippet drops the gauge entirely and shows
`$ctx` plus `$ctxb` if you prefer plain text.

`bin/ctx-bar.sh` renders the gauge and the sparkline and is sourced by the
others. `HERDR_CTX_BAR_CELLS` sets the gauge width (default 10) and
`HERDR_CTX_BAR_STYLE=ascii` swaps to `#`/`.` for fonts without Block Elements
coverage.

## Install

```sh
./install.sh
```

**Nothing restarts** — not herdr, not the server, not your panes, not a single
running agent:

- `settings.json` is never touched, because it already runs that script path,
  so no Claude Code session reloads.
- The statusline is a separate process Claude Code spawns on every render, so a
  running agent picks up the new one on its next redraw. Install writes to a
  temp file and renames, so a render landing mid-install reads either the old
  file or the new one, never a truncated one.
- `HERDR_PANE_ID` is already in every running pane's environment, so existing
  agents start reporting without being restarted.
- `herdr server reload-config` applies the sidebar rows, the widths and the
  tab-row entries live — `apply_live_config` reconfigures `tab_bar_right`
  (`src/app/mod.rs:842`) and `sidebar_agents` (`:848`), and the client reloads
  its own copies (`src/client/shell/config.rs:302-310`).

One thing config cannot move on its own: herdr remembers a manually dragged
sidebar width in `session.json`, and that beats `sidebar_width` from config.
If the sidebar does not widen, drag its right edge — the raised maximum
applies immediately.

It keeps a pristine copy of your statusline in `config/`, timestamps a backup
of anything it replaces, refuses to run if `statusLine` points at some other
script, and refuses to run if the herdr config already defines one of the keys
or tables it would add — or already binds `prefix+y`. Running it twice is safe:
on a second run the marked config ranges are **rewritten in place** rather than
skipped, so a change to `config/herdr-snippet.toml` actually lands, while
anything herdr wrote into `[ui]` itself stays outside those ranges and is
carried over. Hand edits *inside* the markers are replaced; the timestamped
backup has them.

`[[keys.command]]` is deliberately not on the refuse list. It is an array of
tables, so entries from anywhere in the file merge, and your own custom
commands keep working — only the key itself could actually collide.

Missing `xclip` and friends is a warning, not an error: everything except
`prefix+y` works without them, and the popup binding does not need them at
all.

`[ui]` usually already exists — herdr writes into it itself, for instance when
you change a setting in its menu — so the installer merges rather than
refusing: the sub-tables go above the `[ui]` header and the bare keys just
below it. TOML permits a super-table after its sub-tables, and that placement
leaves `[ui]`'s bare-key zone ending outside the block, which is where herdr's
own `upsert_section_raw` (`src/config/io.rs:664`) appends. Verified both ways:
herdr writing `status_indicators`, `agent_panel_sort` and a theme on top of the
installed config keeps it valid, and uninstall then removes only our two marked
ranges and leaves those settings intact.

Requires `jq` and `herdr` on `PATH`.

A session's rows fill in the first time its statusline renders — press a key in
that pane or send it a prompt. Limits appear once any session has rendered.

```sh
./uninstall.sh   # restores your original statusline, strips the config block,
                 # clears reported tokens
```

## Switching the proxy

Edit the `export HTTPS_PROXY=…` lines in `~/.zshrc`, open the **⟳ прокси** tab
and click the button. The tab is one pane running `bin/herdr-proxy-button.py`:

```
  Прокси в ~/.zshrc   127.0.0.1:10808   ● отвечает · 344 мс

          ⟳  Переключить 4 сессии на него          ← a green block, three rows high

  Сессии Claude
  ↻ p1D  BRIEF-tasks-template-public-rest-…   proxy-b.example.com:3128   на старом · работает
  ✓ p2P  tasks rest 3.0 ticket                127.0.0.1:10808                уже на нём
```

herdr 0.8.2 has no configurable buttons: context menus are fixed and tab-row
entries are not clickable (see `upstream/NOTES.md`). What it does do is forward
a left click to a pane app that has turned on mouse reporting, also when that
pane is not focused (`src/app/input/mouse.rs:640` at `v0.8.2`). So the button
is plain terminal output, and a click is an SGR mouse report on its stdin.

A running process cannot be handed a new environment, so a switch is a restart
of each session whose `/proc/<pid>/environ` names another proxy than `~/.zshrc`
(read by `bin/herdr-proxy-upstream.sh`, Claude Code's precedence): `/exit`,
`exec zsh` so the rc is read again, `claude --resume <id>` with the id from
`~/.claude/sessions/<pid>.json`. The conversation survives. A turn in progress
and the session's background shells do not: a busy session is interrupted with
Esc first, because on a dead proxy "working" is usually a retry loop. A session
with typed text in its input box is skipped (the dim suggestion an empty box
shows does not count), and so is one whose claude is not a child of the pane's
shell. A resumed session is checked again; if it still came up on the old
proxy, the cycle runs once more with a longer pause.

Before restarting anything the button checks the new proxy with a TLS
handshake to `api.anthropic.com` through it — a local proxy client answers
`CONNECT` with 200 before it reaches its own upstream, so the status line alone
proves nothing. If that fails the button turns yellow and asks; a second click
switches anyway. The check repeats every minute and whenever `~/.zshrc`
changes, and the tab label counts what is left: `⟳ прокси · 3 на старом`.

herdr restores the tab after a server restart but not the program in it; start
it again in that pane with `~/herder-botox/bin/herdr-proxy-button.py`.

### Without restarts: the relay (not installed)

The alternative never restarts anything, at the cost of one change to Claude
Code's settings. A running process cannot be handed a new environment, so the
switch does not happen inside the sessions at all. `./proxy-install.sh` sets
`HTTPS_PROXY`/`HTTP_PROXY` in the `env` block of `~/.claude/settings.json` to a
local relay, `http://127.0.0.1:18118`, once; the relay
(`bin/herdr-proxy-relay.py`, stdlib Python under a systemd user unit) is the
only thing that knows the real upstream. It reads it through
`bin/herdr-proxy-upstream.sh`, which evaluates just the proxy export lines of
`~/.zshrc` in a bare `zsh -f` — commented alternatives skipped, Claude Code's
own precedence (`https_proxy`, `HTTPS_PROXY`, `http_proxy`, `HTTP_PROXY`). No
proxy line means the relay connects directly.

On `prefix+u` (`bin/herdr-proxy-reload.sh` → `systemctl --user reload`) the
relay re-reads that, **drops every open tunnel** so nothing stays stuck on a
dead upstream, and checks the new one with a `CONNECT api.anthropic.com:443`.
Sessions see a reset connection and retry through the new upstream. Upstream
credentials are injected by the relay and stay in its memory: not in the
status file, not in the journal.

The tab row is silent unless there is something to say:

```
⇄ proxy-a.example …                    checking the new upstream
⇄ proxy-a.example ✓                    a few seconds after the switch, or recovery
⇄ ✗ proxy-b.example.com: 503   the upstream fails; re-checked every 30s
⇄ relay не запущен                 sessions point at a relay that is gone
```

An upstream counts as failing when it cannot be reached, answers 407, or
refuses the API host itself; another host being down behind it does not.

The installer starts the relay and checks the upstream **before** it touches
`settings.json`, and stops there if the API is unreachable — pointing sessions
at it then would cost them the keep-alive connections they still have.
`HERDR_PROXY_FORCE=1` overrides. It refuses to replace a proxy `settings.json`
already sets, keeps timestamped backups of both files it edits, and puts its
herdr block (`config/herdr-proxy-snippet.toml`) right above `[ui]`, between its
own markers.

Running sessions should move over by themselves: on a `settings.json` change
Claude Code re-applies the `env` block and re-runs its proxy-agent setup
(`applyConfigEnvironmentVariables` → global dispatcher, read from the 2.1.312
binary, not from documentation). `bin/herdr-proxy-sessions.sh` shows, per agent
pane, where each claude process's open connections go: `relay`, `мимо relay`
(still on an upstream directly — `/exit` and `claude --resume <id>` from a fresh
shell), or `нет соединений` for an idle one whose next request decides.

```sh
./proxy-install.sh                 # HERDR_PROXY_PORT to change 18118
bin/herdr-proxy-reload.sh          # same as prefix+u; in a terminal it waits and prints the result
bin/herdr-proxy-sessions.sh
journalctl --user -u herdr-proxy-relay
./proxy-uninstall.sh               # settings.json first, the relay last
```

Only `http://` upstreams are relayed (with or without basic auth); a `socks5://`
line shows as `не http-прокси` and the relay keeps the previous upstream.
Plain-HTTP requests are forwarded one per connection, which keeps credential
injection trivially right.

## Caveats

- **Claude Code only.** Other agents need their own reporter; the tokens, gauge
  and sidebar layout are agent-agnostic, the source of the numbers is not. See
  `upstream/NOTES.md` for the manifest-capture patch that would fix this for
  every agent at once.
- **The tab row is monochrome by design.** herdr strips control characters from
  status-command output, and `ESC` is one, so ANSI color cannot survive there.
  Color is only available in sidebar rows, through inline token styles.
- **Gauge glyphs are East Asian "Ambiguous" width.** herdr measures them as one
  column each (`unicode-width`), which is right for the usual case, but a
  terminal using a CJK font may render them double-width and overflow the row.
  Switch to `HERDR_CTX_BAR_STYLE=ascii` if that happens.
- **A limits snapshot older than an hour is dropped, not shown.** A stale
  "you have plenty left" is worse than showing nothing. Remaining percentages
  round pessimistically, so the number never overstates what is left.
  `resets_at` is optional in the payload; the countdown simply disappears when
  it is absent or already past. Only `five_hour` and `seven_day` are read, as
  in your script — the payload can also carry `seven_day_opus` and
  `seven_day_sonnet`.
- **An idle session that has never rendered shows its label but no gauge.**
  Labels are seeded at install; the gauge needs one render from that session.
  Any agent that does a turn fills it in and it stays.
- **Tokens are reported without a TTL**, so they persist until replaced or the
  pane closes. A pane where Claude exited keeps its last gauge until you close
  it. Adding `--ttl-ms` instead makes the rows vanish during long idle periods,
  since the statusline only re-renders when Claude Code redraws — pick your
  poison in `bin/statusline-command.sh`.
- **The rollup sums absolute tokens** and reports the peak percent. Summing
  percentages across sessions would be meaningless; different sessions can also
  have different window sizes. The sparkline compresses at the top: with eight
  height levels, 94% and up all render as a full cell.
- `ui.tab_bar_right` commands run via `/bin/sh -lc`, i.e. a login shell and
  not your `$SHELL` (`src/platform/linux.rs:96`) — every second for the pulse
  and the copy marker, every 5s for the rollup, every 30s for the limits.
  Raise `interval_seconds` if that is heavy on your setup.
- **Showing the name in full costs a row, and two tokens were dropped to pay
  for it.** A session with a two-line name is 5 rows; at six sessions that is
  29 lines against the ~29-30 the Agents panel gets at a 65-row terminal and a
  0.5 section split — it fits with no spare. So `agent` left the first row
  (`◐ claude · Opus 5 (1M context) · xhigh` is 38 columns against 33, and herdr
  was squeezing every token on it; the model name already says which agent it
  is) and `terminal_title_stripped` left the layout. The snippet keeps a
  commented layout that puts the title back at the cost of one more row, and
  `HERDR_TABNAME_LINES=1` returns to a single truncated line. More vertical
  room comes from dragging the divider between Spaces and Agents — that split
  is mouse-only, see `upstream/NOTES.md`.
- **A wrapped name is one poll behind a sidebar drag.** The wrap width is
  computed when the reporter runs, so after dragging the sidebar the rows are
  wrapped for the old width for up to 5 seconds. Nothing breaks meanwhile:
  too-narrow rows truncate with `…` and too-wide ones just leave a gap.
- **No right-click menu offers a copy, and the Agents panel has no right-click
  at all.** Menus open over a Spaces row, a tab in the tab row, and a pane
  (`src/client/shell/mouse.rs:1719-1749`); for a tab the items are `New tab` /
  `Rename` / `Close`. `hits.agents` is read only in the left-button branch,
  where it focuses the pane, so right-clicking an agent row cannot open
  anything. Menu items are not configurable and mouse gestures cannot be
  bound — hence the keybinding. Clicking an agent row focuses its pane, which
  switches the active tab, so click-then-`prefix+y` copies that session's
  name. `upstream/NOTES.md` has the ~15-line patch that would add `Copy name`
  properly.
- **Over `herdr --remote`, `prefix+y` only fires with
  `--remote-keybindings server`.** The default is `local`, which sets the
  keybinding source to `RemoteLocal` — and that mode deliberately *clears*
  custom commands (`src/client/shell/config.rs:283`), since the command would
  otherwise run on your laptop instead of the host the panes live on. With
  `server` the binding comes from the endpoint and runs there. Attached
  locally, a live `reload-config` reloads keybinds in place
  (`apply_live_config`, `src/client/shell/config.rs:266`), so the binding
  arrived without restarting the client. Running the script by hand inside a
  pane works in every case.
- **A custom command bound to a key that already has an action never fires**,
  and nothing warns you: built-in actions are resolved first and custom
  commands only if none matched (`resolve_exact_binding`,
  `src/input/keybindings.rs:221`). `herdr config check` reports a *malformed*
  key but calls a shadowed one `ok` — verified by pointing this binding at
  `prefix+c`, which is `new_tab`. `prefix+y` is unbound in stock herdr.
