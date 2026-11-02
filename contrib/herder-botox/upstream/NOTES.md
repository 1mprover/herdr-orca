# What actually needs a herdr code change

Line references are against `herdrdev/herdr` @ `e7d8220`, the tip of the
default branch. Its `Cargo.toml` also says `0.8.2`, but it is **ahead of the
released 0.8.2 binary installed here** — the `machine` sidebar token,
conditional token `rules` and `keys.command[].description` all exist at this
commit and not in the binary. Every claim below about what the *installed*
herdr does was checked against the binary itself, not only against this tree.
See the README for how that drift bit once already.

Each of your ideas splits cleanly into "already configurable" and "genuinely
missing". The `../` implementation covers the first half with stock config.
This file is the fork/PR list for the second half.

---

## Idea 1 — expanding the Agents panel

### Width: already there, no patch needed
`ui.sidebar_max_width` defaults to 36 (`src/config/model.rs:908`, default at
`:1163`) but `validated_sidebar_bounds` (`src/config/model.rs:300`) only
rejects `min > max` — there is **no upper ceiling**. The upstream test at
`src/config/model.rs:1687` asserts `(0, u16::MAX)` is a valid pair, and `:1642`
exercises `sidebar_max_width = 80`. Raising it in config is supported
behaviour, and the drag handler clamps to the configured pair
(`src/client/shell/mouse.rs:7-20`). Reload picks it up live
(`src/client/shell/config.rs:303-304`).

### Vertical share: this is the real gap
The sidebar splits into Spaces (top) and Agents (bottom). The split ratio is:

- **persisted** — `sidebar_section_split: Option<f32>` (`src/persist/snapshot.rs:26`)
- **mouse-drag only** — `set_sidebar_section_from_row` (`src/client/shell/mouse.rs:22-34`)
- **hard-clamped to 0.1–0.9** — `src/client/shell/mouse.rs:29`, and again on
  restore at `src/client/shell/state.rs:1029`
- **absent from config** — nothing named `section_split` exists under `src/config/`
- **absent from keybindings** — `src/config/keybinds.rs:371` has only
  `toggle_sidebar`; there is no grow/shrink action for either panel

So the Agents panel can never exceed 90% of sidebar height, can never be given
the whole sidebar, and cannot be sized without a mouse or reproducibly from
config.

**Patch shape**

1. Add `ui.sidebar_section_split: Option<f32>` to `UiConfig`
   (`src/config/model.rs:903`+) as the initial value when no persisted
   preference exists, validated in `Config::diagnostics` next to the existing
   sidebar-bounds check (`src/config.rs:147`).
2. Relax the clamp to `0.0..=1.0` at both sites so `1.0` gives Agents the whole
   sidebar and `0.0` gives it to Spaces. The two panels already render from
   independent rects, so a zero-height section is a layout question, not a
   correctness one — check `src/ui/sidebar.rs:51-75`, which already guards
   `content.height < 6`.
3. Add `keys.sidebar_grow_agents` / `keys.sidebar_shrink_agents` actions
   alongside `toggle_sidebar` (`src/config/keybinds.rs:371`, wired at `:686`),
   stepping the ratio and setting `sidebar_section_split_manual`.

Low risk, self-contained, and the kind of thing upstream tends to take.

### Also missing: a row can never wrap
`resolved_token_spans` (`src/ui/sidebar.rs:106`) hands every flexible token a
budget out of one row's width and truncates each with `truncate_end`
(`src/ui/text.rs:7`); rows come straight from config and nothing folds a long
value onto the next one. At the stock 26-column sidebar a continuation row has
21 usable columns — the width, minus the divider, minus the 3-column indent on
every row after the first (`src/client/shell/agent_sidebar.rs:357`), minus the
scrollbar — so any tab name past about 20 characters is unreadable in the
Agents panel no matter how the rows are arranged. Generated task slugs
(`BRIEF-checklist-participants-click-2026-09-02`) are exactly that case.

`../bin/herdr-tabname-report.sh` works around it by wrapping the name outside
herdr and reporting the pieces as separate tokens, one per row, which means the
workaround has to guess the sidebar width and re-guess it whenever the sidebar
is dragged.

**Patch shape** — a per-row option, evaluated where the budgets are already
known:

```toml
rows = [["state_icon", "workspace"], [{ token = "tab", wrap = 2 }]]
```

`wrap = n` lets one token spill onto up to `n` extra lines instead of being
truncated, which the row loop can express directly: it already renders one
`Paragraph` per configured row (`src/client/shell/agent_sidebar.rs:357-373`)
and `row_lines` already reports a variable height per entry
(`:82`). Everything needed is in scope; only the split is missing.

---

## Idea 2 — context usage per session

### The display plumbing exists
Custom tokens are a first-class sidebar feature: `AgentSidebarToken::Custom`
resolves from a per-pane map (`src/ui/sidebar/tokens.rs:106-110`), fed by
`pane.report_metadata`, and `herdr agent list` returns those tokens
(socket API docs, `docs/next/website/src/content/docs/socket-api.mdx:778`).
That is what `../bin/herdr-ctx-report.sh` uses, and it needs no patch.

### What is missing: a native, agent-agnostic context token
Today every agent needs its own external reporter. herdr already scrapes each
pane's visible text for state detection, and several agents print context usage
right there — the bundled Devin fixtures literally contain
`Context: 16k / 200k tokens (7%)` (`src/detect/manifest/tests.rs:307`, `:314`).
But detection rules are classify-only: a rule yields a `state`, a priority and
guards, with no capture output (`src/detect/manifests/*.toml`, parsed in
`src/detect/manifest.rs`). The regex crate herdr already depends on supports
named captures; nothing consumes them.

**Patch shape**

Add a `[[metrics]]` section to the manifest format, evaluated over the same
regions as `[[rules]]` (`src/detect/manifest.rs:1290-1356` resolves
`osc_title`, `bottom_non_empty_lines(n)`, etc.):

```toml
[[metrics]]
id = "context_window"
region = "bottom_non_empty_lines(3)"
regex = ['Context:\s*(?<used>\d+)k\s*/\s*(?<total>\d+)k\s*tokens\s*\((?<pct>\d+)%\)']
token = "context"
format = "{pct}"
```

Populate the same per-pane token map that `report_metadata` writes, under a
reserved namespace so a manifest cannot clobber a user token. `$context` then
works in sidebar rows for every agent whose TUI shows it, with the existing
numeric `rules` for thresholds, and no per-agent hook.

Note the honest limit: **Claude Code does not print context usage in its
footer** — `src/detect/manifests/claude.toml` has no context pattern because
there is nothing to match. For Claude the statusLine hook in `../bin/` stays
the right source either way; it is exact rather than scraped.

### Also missing: a fleet rollup token
Space rows roll up child agent state and git counts, but `agent_rows`
(`src/ui/sidebar/tokens.rs:67`) receives one pane's context
(`AgentTokenContext`, `:54-64`) and has no view of siblings, so no sum or max
across sessions can be expressed in sidebar config. `../bin/herdr-ctx-total.sh`
works around this through `ui.tab_bar_right` + `herdr agent list`, which costs a
process spawn every 5s.

**Patch shape** — a `$sum:<token>` / `$max:<token>` form resolved in the Space
rollup, where the child list is already in hand.

---

## Idea 3 — copying a tab name

### The clipboard is one function call away from the CLI
herdr's clipboard path is finished and works in both directions for panes: a
pane emits OSC 52, `capture_clipboard_write` picks it up through libghostty-vt
(`src/ghostty/mod.rs:568`), the read loop queues
`AppEvent::ClipboardWrite { content }` (`src/pane.rs:2183` and `:2379`), the
server ships `ServerMessage::Clipboard`, and the client base64-decodes it,
re-emits OSC 52 to the outer terminal and raises its own copy toast
(`src/client/clipboard_forwarding.rs:9`, `src/client/mod.rs:1724`).

Nothing but a pane can start it. There is no `herdr clipboard set`; `pane
send-text` and `pane run` write to a pane's *input*, which for an agent pane
types into the agent's prompt. So anything herdr itself knows and displays —
tab names, workspace names, pane ids, branch names, an agent's state label —
is unreachable from a script, and unreachable from the `type = "shell"`
keybinding herdr offers for exactly this sort of glue, because that is spawned
with all three descriptors on `/dev/null` (`src/app/custom_commands.rs:288`).

**Patch shape** — a CLI verb that emits the event the machinery already
consumes:

```
herdr clipboard set [--text TEXT | --stdin]
```

One socket method that constructs `AppEvent::ClipboardWrite` from its argument.
Every attached client then gets the same forwarding and the same toast that a
pane's OSC 52 gets today, including the remote case, where the copy has to land
on the machine the human is attached from rather than the one the panes run on.
`../bin/herdr-copy-tab.sh` currently needs `wl-copy`/`xclip`/`pbcopy` to work
around this, which copies on the *server's* host — wrong for `herdr --remote` —
and gets no toast.

### Smaller, related: no menu anywhere offers a copy
Right-click opens a context menu over three regions, in this dispatch order
(`src/client/shell/mouse.rs:1719-1749`): a workspace row in Spaces, a tab in
the tab row, then a pane. Their complete item sets are in
`src/client/shell/context_menu.rs:8-77` — for a tab, `New tab` / `Rename` /
`Close`.

Two gaps fall out of that:

1. **An Agents-panel row has no right-click behaviour at all.** `hits.agents`
   (`src/client/shell/state.rs:152`) is consulted only in the left-button
   branch, where it emits `PaneFocus` (`src/client/shell/mouse.rs:2032`). The
   right-button branch never looks at it, so right-clicking an agent — the row
   that displays the tab name — cannot open anything. The 0.8.2 docs say
   "click panes, tabs, workspaces, and agents" and, separately, "use
   right-click menus"; the four clickable things and the three right-clickable
   ones are not the same list.
2. **No menu has a copy item.** In the installed binary the context-menu
   labels are one contiguous blob — `Rename`, `Close`, `New worktree`,
   `Expand`, `Close group`, `Delete worktree checkout...`, `New tab`,
   `Rename pane`, `Clear pane name`, `Swap with focused pane`, `Split right`,
   `Split down`, `Send right-clicks to pane`, `Use Herdr right-click menu`,
   `Close pane` — and none of them copies anything. Selection copy
   (`copy_on_select`, double-click token copy) covers pane *content* only;
   sidebar and tab-row text is drawn from state, so there is nothing to select.
   Tab-row status entries are not clickable either: the hit map has no region
   for them (`src/client/shell/state.rs:140-195`).

**Patch shape** — the client already owns both halves, so this is small:
`write_osc52_bytes` (`src/selection.rs:312`) tries the native clipboard first
and falls back to OSC 52, and `show_copy_feedback` raises the existing "copied"
toast (`src/client/mod.rs:1729`). Add `item("Copy name", Action::CopyName)` to
the Tab, Pane and Workspace arms, handle it by passing the label already in the
menu target to those two calls, and expose the same thing as a
`keys.copy_tab_name` action. No new plumbing, no socket method, and it fixes
the mouse-first expectation rather than routing around it — a right-click on an
Agents row would still need gap 1 closed as well.

### Also worth fixing: a shadowed custom command is silent
`resolve_exact_binding` (`src/input/keybindings.rs:221`) resolves built-in
actions first and reaches `resolve_custom_command` only if none matched, so a
`[[keys.command]]` entry bound to a key that already has an action never runs.
`herdr config check` calls that config `ok` — verified with `key = "prefix+c"`,
which is `new_tab`. It diagnoses a malformed key (`invalid keybinding:
keys.command[0].key = "prefix+"; disabling binding`) but not a shadowed one.
`append_custom_command_bindings` already has the `BindingRegistry` and a
`diagnostics` vec in hand (`src/config/keybinds.rs:751`); one lookup there
turns a silent no-op into a warning.

---

## Packaging

If you want this shareable rather than a private fork, herdr's plugin system is
the intended vehicle: a `herdr-plugin.toml` with a build step and a long-running
command, and "the entire Herdr CLI is the plugin API"
(`docs/next/website/src/content/docs/plugins.mdx`). A plugin could own the
reporter for every supported agent and drop the statusLine wrapper entirely —
worth doing only after the manifest-capture patch above, otherwise it is just
the same hook in a nicer box.
