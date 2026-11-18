//! The card sidebar: a stable list of workspace cards. Each card shows its lead
//! agent's state, its branch and its sessions: agents with what they are doing
//! and for how long, then plain terminals. The focused card sits in a frame and
//! lists every session that fits. Pinned cards come first, the header counts
//! agents by state, and an optional status line sits above the footer.
//!
//! Cards never move because an agent changed state: the order is the
//! workspace order (or repository sections), so a card stays where you left
//! it.

use std::time::SystemTime;

use super::*;
use crate::api::schema::AgentStatus;
use crate::config::{CardsGroupBy, CardsSidebarConfig};
use crate::protocol::ClientShellAgent;
use crate::ui::truncate_end;

use super::cards_activity::{format_age, AgentActivityClock};
use super::cards_rows::{agent_glyph, count_label, row_window, SessionIndex, SessionKind};
use super::render::{
    display_width, put_right_text, put_segment, put_text, render_sidebar_background,
    ShellRenderState,
};

/// Section collapse state shares `collapsed_groups` with worktree groups, so
/// card sections carry a prefix that no repository key can start with.
pub(super) const SECTION_KEY_PREFIX: &str = "cards:";
/// A card's folded session list, keyed `cards:sessions:<workspace_id>` in
/// `collapsed_groups`.
pub(super) const SESSIONS_KEY_PREFIX: &str = "cards:sessions:";
/// The search field, a blank row, the title with its counters, a blank row.
const HEADER_ROWS: u16 = 4;
const SUMMARY_STATUSES: [AgentStatus; 3] = [
    AgentStatus::Blocked,
    AgentStatus::Done,
    AgentStatus::Working,
];

pub(super) struct CardsView<'a> {
    pub(super) group_by: CardsGroupBy,
    pub(super) collapsed_groups: &'a HashSet<String>,
    pub(super) pinned: &'a [String],
    pub(super) filter: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CardItem {
    Section {
        key: String,
        label: String,
        count: usize,
        collapsed: bool,
    },
    /// The blank row between pinned cards and the rest.
    Divider,
    Card {
        index: usize,
    },
}

fn status_key(status: AgentStatus) -> (&'static str, &'static str) {
    match status {
        AgentStatus::Blocked => ("blocked", "waiting"),
        AgentStatus::Done => ("done", "done"),
        AgentStatus::Working => ("working", "working"),
        AgentStatus::Idle | AgentStatus::Unknown => ("idle", "idle"),
    }
}

fn status_word(cards: &CardsSidebarConfig, status: AgentStatus) -> &str {
    let (key, fallback) = status_key(status);
    cards.label(key, fallback)
}

pub(super) fn workspace_matches(
    snapshot: &ClientShellSnapshot,
    workspace: &ClientShellWorkspace,
    needle: &str,
) -> bool {
    let hit = |text: &str| text.to_lowercase().contains(needle);
    let id = workspace.workspace_id.as_str();
    hit(&workspace.label)
        || workspace.branch.as_deref().is_some_and(hit)
        || workspace
            .worktree
            .as_ref()
            .is_some_and(|worktree| hit(&worktree.label))
        || snapshot
            .tabs
            .iter()
            .any(|tab| tab.workspace_id == id && hit(&tab.label))
        || snapshot
            .panes
            .iter()
            .any(|pane| pane.workspace_id == id && hit(&super::cards_rows::pane_label(pane)))
        || snapshot
            .agents
            .iter()
            .filter(|agent| agent.workspace_id == id)
            .any(|agent| super::cards_rows::agent_matches(agent, needle))
}

/// Cards in display order: pinned first, then workspace order or one section
/// per repository. A collapsed section keeps the focused card visible, and a
/// non-empty filter shows every match regardless of collapse.
pub(super) fn card_items(
    snapshot: &ClientShellSnapshot,
    cards: &CardsSidebarConfig,
    view: &CardsView<'_>,
) -> Vec<CardItem> {
    let needle = view.filter.trim().to_lowercase();
    let filtering = !needle.is_empty();
    let visible = |index: usize| {
        !filtering || workspace_matches(snapshot, &snapshot.workspaces[index], &needle)
    };

    let mut pinned = Vec::new();
    for id in view.pinned {
        if let Some(index) = snapshot
            .workspaces
            .iter()
            .position(|workspace| &workspace.workspace_id == id)
        {
            if visible(index) && !pinned.contains(&index) {
                pinned.push(index);
            }
        }
    }
    // The classic list's order keeps linked worktrees next to their repository.
    let rest = super::sidebar::workspace_entries(snapshot, &HashSet::new())
        .into_iter()
        .map(|entry| entry.index)
        .filter(|index| {
            !view
                .pinned
                .contains(&snapshot.workspaces[*index].workspace_id)
                && visible(*index)
        })
        .collect::<Vec<_>>();

    let mut items = pinned
        .iter()
        .map(|index| CardItem::Card { index: *index })
        .collect::<Vec<_>>();
    if !pinned.is_empty() && !rest.is_empty() {
        items.push(CardItem::Divider);
    }
    match view.group_by {
        CardsGroupBy::None => items.extend(rest.into_iter().map(|index| CardItem::Card { index })),
        CardsGroupBy::Project => {
            let mut projects: Vec<(String, String, Vec<usize>)> = Vec::new();
            for index in rest {
                let worktree = snapshot.workspaces[index].worktree.as_ref();
                let key = worktree.map_or("", |worktree| worktree.key.as_str());
                if let Some(project) = projects.iter_mut().find(|project| project.0 == key) {
                    project.2.push(index);
                } else {
                    let label = worktree.map_or_else(
                        || cards.label("no_repo", "no repo").to_owned(),
                        |worktree| worktree.label.clone(),
                    );
                    projects.push((key.to_owned(), label, vec![index]));
                }
            }
            // Workspaces outside any repository go last.
            projects.sort_by_key(|project| project.0.is_empty());
            for (key, label, members) in projects {
                let key = format!("{SECTION_KEY_PREFIX}project:{key}");
                let collapsed = !filtering && view.collapsed_groups.contains(&key);
                items.push(CardItem::Section {
                    key,
                    label,
                    count: members.len(),
                    collapsed,
                });
                items.extend(
                    members
                        .into_iter()
                        .filter(|index| !collapsed || snapshot.workspaces[*index].focused)
                        .map(|index| CardItem::Card { index }),
                );
            }
        }
    }
    items
}

/// Workspace indices in the order their cards appear.
pub(super) fn card_workspace_order(
    snapshot: &ClientShellSnapshot,
    cards: &CardsSidebarConfig,
    view: &CardsView<'_>,
) -> Vec<usize> {
    card_items(snapshot, cards, view)
        .into_iter()
        .filter_map(|item| match item {
            CardItem::Card { index } => Some(index),
            CardItem::Section { .. } | CardItem::Divider => None,
        })
        .collect()
}

/// What the agent is doing: its terminal title, else its state in words.
fn agent_activity<'a>(cards: &'a CardsSidebarConfig, agent: &'a ClientShellAgent) -> &'a str {
    agent
        .terminal_title_stripped
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| status_word(cards, agent.agent_status))
}

/// The agent a card speaks for: the most urgent state, then the most recent
/// change.
fn lead_agent<'a>(
    agents: &[&'a ClientShellAgent],
    clock: &AgentActivityClock,
) -> Option<&'a ClientShellAgent> {
    agents.iter().copied().max_by(|left, right| {
        status_priority(left.agent_status)
            .cmp(&status_priority(right.agent_status))
            .then_with(|| clock.since(&left.pane_id).cmp(&clock.since(&right.pane_id)))
    })
}

/// Shows a path relative to the home directory as `~/...`.
fn display_path(path: &str, home: Option<&str>) -> String {
    match home.filter(|home| !home.is_empty()) {
        Some(home) if path == home => "~".to_owned(),
        Some(home) => path
            .strip_prefix(home)
            .filter(|rest| rest.starts_with('/'))
            .map_or_else(|| path.to_owned(), |rest| format!("~{rest}")),
        None => path.to_owned(),
    }
}

type Segment = (String, Style);

struct CardLine {
    /// One-cell marks at fixed columns, as offsets from the card's left edge.
    marks: Vec<(u16, Segment)>,
    /// Where the text starts, as an offset from the card's left edge.
    indent: u16,
    left: Vec<Segment>,
    right: Vec<Segment>,
    /// Session rows focus their pane; every other line selects the workspace.
    pane_id: Option<String>,
    /// The counter row folds the card's session list.
    toggle: bool,
    /// The focused pane's row.
    highlight: bool,
}

impl CardLine {
    fn text(indent: u16, left: Vec<Segment>, right: Vec<Segment>) -> Self {
        Self {
            marks: Vec::new(),
            indent,
            left,
            right,
            pane_id: None,
            toggle: false,
            highlight: false,
        }
    }
}

/// Card columns: the frame, the workspace status, the text, and the agent
/// glyph and title of session rows.
const FRAME_COLUMN: u16 = 1;
const STATUS_COLUMN: u16 = 3;
const TEXT_COLUMN: u16 = 5;
const GLYPH_COLUMN: u16 = 7;
const TITLE_COLUMN: u16 = 9;
/// The frame, the name, branch and counter rows, and the "… more N" row a
/// focused card needs besides its session rows.
const FOCUSED_CARD_CHROME: usize = 6;

struct BuiltCard {
    index: usize,
    lines: Vec<CardLine>,
    /// The focused card sits in a frame.
    framed: bool,
}

enum BuiltItem {
    Section(CardItem),
    Divider,
    Card(BuiltCard),
}

impl BuiltItem {
    fn height(&self) -> u16 {
        match self {
            Self::Section(_) | Self::Divider => 1,
            Self::Card(card) => {
                let frame = if card.framed { 2 } else { 0 };
                (card.lines.len() + frame).clamp(1, u16::MAX as usize) as u16
            }
        }
    }
}

struct CardContext<'a> {
    snapshot: &'a ClientShellSnapshot,
    config: &'a ClientShellConfig,
    clock: &'a AgentActivityClock,
    now: SystemTime,
    home: Option<&'a str>,
    sessions: &'a SessionIndex<'a>,
    /// Session rows the focused card may show and still fit in the list.
    focused_limit: usize,
    collapsed_groups: &'a HashSet<String>,
    /// A filter shows every card's sessions, folded or not.
    filtering: bool,
}

fn build_card(context: &CardContext<'_>, index: usize) -> Option<BuiltCard> {
    let CardContext {
        snapshot,
        config,
        clock,
        now,
        home,
        sessions,
        focused_limit,
        collapsed_groups,
        filtering,
    } = context;
    let palette = &config.palette;
    let cards = &config.cards;
    let workspace = snapshot.workspaces.get(index)?;
    let id = workspace.workspace_id.as_str();
    let focused = workspace.focused;
    let agents = snapshot
        .agents
        .iter()
        .filter(|agent| agent.workspace_id == id)
        .collect::<Vec<_>>();
    let lead = lead_agent(&agents, clock);
    let dim = Style::default().fg(palette.overlay0);
    let status_mark = |status: AgentStatus| {
        (
            status_icon(status, config.status_indicators).to_owned(),
            Style::default().fg(status_color(status, palette)),
        )
    };
    let mut lines = Vec::new();

    let lead_status = lead.map_or(AgentStatus::Unknown, |lead| lead.agent_status);
    let mut name = CardLine::text(
        TEXT_COLUMN,
        vec![(
            workspace.label.clone(),
            Style::default()
                .fg(if focused {
                    palette.text
                } else {
                    palette.subtext0
                })
                .add_modifier(Modifier::BOLD),
        )],
        Vec::new(),
    );
    name.marks.push((STATUS_COLUMN, status_mark(lead_status)));
    lines.push(name);

    match workspace.branch.as_deref() {
        Some(branch) => {
            let mut ahead_behind = Vec::new();
            if let Some((ahead, behind)) = workspace.git_ahead_behind {
                if ahead > 0 {
                    ahead_behind.push(format!("↑{ahead}"));
                }
                if behind > 0 {
                    ahead_behind.push(format!("↓{behind}"));
                }
            }
            let right = if ahead_behind.is_empty() {
                Vec::new()
            } else {
                vec![(ahead_behind.join(" "), dim)]
            };
            lines.push(CardLine::text(
                TEXT_COLUMN,
                vec![
                    ("⎇ ".to_owned(), Style::default().fg(palette.mauve)),
                    (
                        branch.to_owned(),
                        Style::default().fg(if focused {
                            palette.overlay1
                        } else {
                            palette.overlay0
                        }),
                    ),
                ],
                right,
            ));
        }
        None => {
            let pane = snapshot
                .panes
                .iter()
                .filter(|pane| pane.tab_id == workspace.active_tab_id)
                .max_by_key(|pane| pane.focused);
            let path = pane
                .and_then(|pane| pane.foreground_cwd.as_deref().or(pane.cwd.as_deref()))
                .unwrap_or(&workspace.new_workspace_cwd);
            lines.push(CardLine::text(
                TEXT_COLUMN,
                vec![(display_path(path, *home), dim)],
                Vec::new(),
            ));
        }
    }

    let sessions = sessions.sessions(id);
    let folded = !*filtering && collapsed_groups.contains(&format!("{SESSIONS_KEY_PREFIX}{id}"));
    if focused {
        let mut counts = Vec::new();
        if sessions.agents > 0 || sessions.terminals == 0 {
            counts.push(count_label(
                cards,
                "agents",
                sessions.agents,
                ["agent", "agents", "agents"],
            ));
        }
        if sessions.terminals > 0 {
            counts.push(count_label(
                cards,
                "terminals",
                sessions.terminals,
                ["terminal", "terminals", "terminals"],
            ));
        }
        let mut counter = CardLine::text(
            TEXT_COLUMN,
            vec![(
                format!("{} {}", if folded { "▸" } else { "▾" }, counts.join(" · ")),
                Style::default().fg(palette.overlay1),
            )],
            Vec::new(),
        );
        counter.toggle = true;
        lines.push(counter);
    }

    let focused_row = sessions.rows.iter().position(|row| row.focused);
    let window = if folded {
        row_window(0, 0, None)
    } else if focused {
        row_window(sessions.rows.len(), *focused_limit, focused_row)
    } else {
        row_window(sessions.rows.len(), usize::from(cards.inactive_rows), None)
    };
    for row in sessions.rows.iter().skip(window.start).take(window.len) {
        let highlight = focused && row.focused;
        let title_style = if highlight {
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.subtext0)
        };
        let mut line = match &row.kind {
            SessionKind::Agent(agent) => {
                let right = clock
                    .since(row.pane_id)
                    .map(|since| vec![(format_age(cards, since, *now), dim)])
                    .unwrap_or_default();
                let mut line = CardLine::text(
                    TITLE_COLUMN,
                    vec![(agent_activity(cards, agent).to_owned(), title_style)],
                    right,
                );
                let (glyph, color) = agent_glyph(agent, palette);
                line.marks
                    .push((TEXT_COLUMN, status_mark(agent.agent_status)));
                line.marks
                    .push((GLYPH_COLUMN, (glyph.to_owned(), Style::default().fg(color))));
                line
            }
            SessionKind::Terminal {
                tab_label,
                pane_label,
                tab_has_one_pane,
            } => {
                let mut left = vec![((*tab_label).to_owned(), title_style)];
                if !(*tab_has_one_pane && pane_label == tab_label) {
                    left.push((format!(" · {pane_label}"), dim));
                }
                let mut line = CardLine::text(TITLE_COLUMN, left, Vec::new());
                line.marks.push((
                    GLYPH_COLUMN,
                    ("$".to_owned(), Style::default().fg(palette.overlay1)),
                ));
                line
            }
        };
        line.pane_id = Some(row.pane_id.to_owned());
        line.highlight = highlight;
        lines.push(line);
    }
    if window.hidden > 0 {
        lines.push(CardLine::text(
            TEXT_COLUMN,
            vec![(
                format!("… {} {}", cards.label("more", "more"), window.hidden),
                dim,
            )],
            Vec::new(),
        ));
    }
    Some(BuiltCard {
        index,
        lines,
        framed: focused,
    })
}

/// Draws left segments, shortening the last one that does not fit, and
/// right-aligns the right segments one column in from the edge.
fn draw_line(buffer: &mut Buffer, rect: Rect, left: &[Segment], right: &[Segment]) {
    let right_width = right
        .iter()
        .map(|(text, _)| display_width(text))
        .sum::<u16>();
    let edge = rect.right().saturating_sub(1);
    let right_start = edge.saturating_sub(right_width);
    let left_limit = if right.is_empty() {
        edge
    } else {
        right_start.saturating_sub(1)
    };
    let mut x = rect.x;
    for (text, style) in left {
        if x >= left_limit {
            break;
        }
        let room = (left_limit - x) as usize;
        let text = truncate_end(text, room);
        x = put_segment(buffer, x, rect.y, left_limit, &text, *style);
    }
    let mut x = right_start.max(rect.x);
    for (text, style) in right {
        x = put_segment(buffer, x, rect.y, edge, text, *style);
    }
}

fn selection_background(palette: &Palette) -> ratatui::style::Color {
    if palette.selection_bg == ratatui::style::Color::Reset {
        palette.active_row_bg
    } else {
        palette.selection_bg
    }
}

fn agent_counts(snapshot: &ClientShellSnapshot) -> [usize; 4] {
    let mut counts = [0; 4];
    for agent in &snapshot.agents {
        let slot = match agent.agent_status {
            AgentStatus::Blocked => 0,
            AgentStatus::Done => 1,
            AgentStatus::Working => 2,
            AgentStatus::Idle | AgentStatus::Unknown => 3,
        };
        counts[slot] += 1;
    }
    counts
}

pub(super) fn render_cards_sidebar(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    state: &mut ShellRenderState<'_>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    let cards = &config.cards;
    render_sidebar_background(buffer, area, palette);
    hits.sidebar_divider = if area.is_empty() {
        Rect::default()
    } else {
        Rect::new(area.right().saturating_sub(1), area.y, 1, area.height)
    };
    hits.sidebar_section_divider = Rect::default();
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.is_empty() {
        return;
    }
    let dim = Style::default().fg(palette.overlay0);

    let field = Rect::new(
        content.x.saturating_add(1),
        content.y,
        content.width.saturating_sub(5),
        1,
    );
    render_search_field(buffer, field, config, state, hits);
    if config.mouse_capture && content.width > 8 {
        hits.card_filter_toggle = field;
        let group = Rect::new(content.right().saturating_sub(3), content.y, 2, 1);
        put_text(
            buffer,
            group.x,
            group.y,
            1,
            "≡",
            Style::default().fg(if cards.group_by == CardsGroupBy::Project {
                palette.accent
            } else {
                palette.overlay0
            }),
        );
        hits.card_group_toggle = group;
    }
    if content.height > 2 {
        let row = Rect::new(content.x, content.y + 2, content.width, 1);
        render_title_row(buffer, row, snapshot, config, hits);
    }

    let footer_y = content.bottom().saturating_sub(1);
    let status_line = state
        .cards_footer
        .filter(|text| !text.is_empty() && content.height > HEADER_ROWS + 3);
    let reserved = 1 + 2 * u16::from(status_line.is_some());
    let body = Rect::new(
        content.x,
        content.y.saturating_add(HEADER_ROWS),
        content.width,
        content.height.saturating_sub(HEADER_ROWS + reserved),
    );
    hits.workspace_body = body;

    let home = std::env::var("HOME").ok();
    let needle = state.sidebar_filter.as_str().trim().to_lowercase();
    let sessions = SessionIndex::new(snapshot, &needle);
    let context = CardContext {
        snapshot,
        config,
        clock: state.cards_clock,
        now: state.cards_now,
        home: home.as_deref(),
        sessions: &sessions,
        focused_limit: usize::from(body.height).saturating_sub(FOCUSED_CARD_CHROME),
        collapsed_groups: state.collapsed_groups,
        filtering: !needle.is_empty(),
    };
    let view = CardsView {
        group_by: cards.group_by,
        collapsed_groups: state.collapsed_groups,
        pinned: state.pinned_workspaces,
        filter: state.sidebar_filter.as_str(),
    };
    let items = card_items(snapshot, cards, &view)
        .into_iter()
        .filter_map(|item| match item {
            CardItem::Card { index } => build_card(&context, index).map(BuiltItem::Card),
            CardItem::Divider => Some(BuiltItem::Divider),
            section => Some(BuiltItem::Section(section)),
        })
        .collect::<Vec<_>>();
    render_card_list(buffer, body, snapshot, config, state, hits, &items);

    if let Some(text) = status_line {
        let rule_y = footer_y.saturating_sub(2);
        put_text(
            buffer,
            content.x,
            rule_y,
            content.width,
            &"─".repeat(content.width as usize),
            Style::default().fg(palette.surface_dim),
        );
        put_text(
            buffer,
            content.x.saturating_add(1),
            footer_y.saturating_sub(1),
            content.width.saturating_sub(2),
            &truncate_end(text, content.width.saturating_sub(2) as usize),
            Style::default().fg(palette.subtext0),
        );
    }

    if config.mouse_capture {
        let new_label = format!(" + {}", cards.label("new", "new"));
        hits.new_workspace = Rect::new(
            content.x,
            footer_y,
            display_width(&new_label).min(content.width),
            1,
        );
        put_text(buffer, content.x, footer_y, content.width, &new_label, dim);
        // Leave the last columns to the collapse toggle.
        let menu_area = Rect::new(content.x, footer_y, content.width.saturating_sub(2), 1);
        let menu = cards.label("menu", "menu");
        let attention = super::global_menu::global_menu_attention(snapshot);
        let menu_width = display_width(menu) + if attention { 2 } else { 0 };
        let launcher_width = (menu_width + 2).min(menu_area.width);
        hits.global_launcher = Rect::new(
            menu_area.right().saturating_sub(launcher_width),
            footer_y,
            launcher_width,
            1,
        );
        if attention {
            put_text(
                buffer,
                menu_area.right().saturating_sub(menu_width),
                footer_y,
                2,
                "● ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            );
        }
        put_right_text(buffer, menu_area, footer_y, menu, dim);
    }

    hits.sidebar_toggle = Rect::new(
        area.right().saturating_sub(2),
        area.bottom().saturating_sub(1),
        u16::from(area.width > 1),
        u16::from(area.height > 0),
    );
    put_text(
        buffer,
        hits.sidebar_toggle.x,
        hits.sidebar_toggle.y,
        hits.sidebar_toggle.width,
        "«",
        dim,
    );
}

/// The search field: a hint while idle, the editor while typing, and the
/// filter text while a filter is set.
fn render_search_field(
    buffer: &mut Buffer,
    field: Rect,
    config: &ClientShellConfig,
    state: &mut ShellRenderState<'_>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    if field.width < 4 {
        return;
    }
    let base = Style::default().bg(palette.surface0);
    put_text(
        buffer,
        field.x,
        field.y,
        field.width,
        &" ".repeat(field.width as usize),
        base,
    );
    let filtering = !state.sidebar_filter.trim().is_empty();
    let active = state.sidebar_filter_editing || filtering;
    put_text(
        buffer,
        field.x.saturating_add(1),
        field.y,
        1,
        "⌕",
        base.fg(if active {
            palette.accent
        } else {
            palette.overlay1
        }),
    );
    let text = Rect::new(
        field.x.saturating_add(4),
        field.y,
        field.width.saturating_sub(5),
        1,
    );
    if state.sidebar_filter_editing {
        hits.card_filter = field;
        hits.card_filter_cursor =
            super::text_editor::render(buffer, text, state.sidebar_filter, base.fg(palette.text));
    } else if filtering {
        hits.card_filter = field;
        put_text(
            buffer,
            text.x,
            text.y,
            text.width,
            state.sidebar_filter.as_str(),
            base.fg(palette.accent),
        );
    } else {
        put_text(
            buffer,
            text.x,
            text.y,
            text.width,
            config.cards.label("search", "search"),
            base.fg(palette.overlay1),
        );
    }
}

/// The title on the left and agent counts by state on the right; each count
/// jumps to the next agent in that state. Counts drop their words when the
/// row is too narrow for them.
fn render_title_row(
    buffer: &mut Buffer,
    row: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    let title = format!(
        " {}",
        config.cards.label("title", "workspaces").to_uppercase()
    );
    put_text(
        buffer,
        row.x,
        row.y,
        row.width,
        &title,
        Style::default()
            .fg(palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );
    let counts = agent_counts(snapshot);
    let mut chips = SUMMARY_STATUSES
        .iter()
        .zip(counts)
        .filter(|(_, count)| *count > 0)
        .map(|(status, count)| (*status, count))
        .collect::<Vec<_>>();
    if chips.is_empty() && counts[3] > 0 {
        chips.push((AgentStatus::Idle, counts[3]));
    }
    let chip_text = |status: AgentStatus, count: usize, words: bool| {
        let icon = status_icon(status, config.status_indicators);
        if words {
            format!("{icon} {count} {}", status_word(&config.cards, status))
        } else {
            format!("{icon} {count}")
        }
    };
    let width_of = |words: bool| {
        chips
            .iter()
            .map(|(status, count)| display_width(&chip_text(*status, *count, words)))
            .sum::<u16>()
            + 2 * chips.len().saturating_sub(1) as u16
    };
    let available = row.width.saturating_sub(display_width(&title) + 3);
    let words = width_of(true) <= available;
    let width = width_of(words);
    if width > available {
        return;
    }
    let mut x = row.right().saturating_sub(1 + width);
    for (status, count) in chips {
        let text = chip_text(status, count, words);
        let start = x;
        x = put_segment(
            buffer,
            x,
            row.y,
            row.right(),
            &text,
            Style::default().fg(status_color(status, palette)),
        );
        if config.mouse_capture && status != AgentStatus::Idle {
            hits.card_status
                .push((Rect::new(start, row.y, x - start, 1), status));
        }
        x = x.saturating_add(2);
    }
}

fn render_card_list(
    buffer: &mut Buffer,
    body: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    state: &mut ShellRenderState<'_>,
    hits: &mut ShellHitMap,
    items: &[BuiltItem],
) {
    let palette = &config.palette;
    if body.is_empty() {
        return;
    }
    if items.is_empty() {
        *state.workspace_scroll = 0;
        let message = if state.sidebar_filter.trim().is_empty() {
            config.cards.label("empty", "no workspaces")
        } else {
            config.cards.label("no_matches", "no matches")
        };
        put_text(
            buffer,
            body.x,
            body.y,
            body.width,
            &format!(" {message}"),
            Style::default()
                .fg(palette.overlay0)
                .add_modifier(Modifier::DIM),
        );
        return;
    }

    let heights = items.iter().map(BuiltItem::height).collect::<Vec<_>>();
    let gaps = items
        .iter()
        .enumerate()
        .map(|(position, item)| match (item, items.get(position + 1)) {
            (BuiltItem::Card(_), Some(BuiltItem::Card(_))) => config.cards.card_gap,
            _ => 0,
        })
        .collect::<Vec<_>>();
    let mut metrics =
        super::scroll::list_scroll_metrics(&heights, &gaps, body.height, *state.workspace_scroll);
    if std::mem::take(state.reveal_focused_workspace) {
        if let Some(target) = items.iter().position(
            |item| matches!(item, BuiltItem::Card(card) if snapshot.workspaces[card.index].focused),
        ) {
            *state.workspace_scroll = super::scroll::list_scroll_start_to_reveal(
                &heights,
                &gaps,
                body.height,
                *state.workspace_scroll,
                target,
            );
            metrics = super::scroll::list_scroll_metrics(
                &heights,
                &gaps,
                body.height,
                *state.workspace_scroll,
            );
        }
    }
    hits.workspace_max_scroll = metrics.max_offset_from_bottom;
    hits.workspace_scroll_metrics = Some(metrics);
    *state.workspace_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let width = body.width.saturating_sub(u16::from(show_scrollbar));

    let mut y = body.y;
    for (position, item) in items.iter().enumerate().skip(*state.workspace_scroll) {
        if y >= body.bottom() {
            break;
        }
        // Cards taller than the space left are clipped rather than skipped.
        let height = heights[position].min(body.bottom() - y);
        let rect = Rect::new(body.x, y, width, height);
        match item {
            BuiltItem::Section(CardItem::Section {
                key,
                label,
                count,
                collapsed,
            }) => {
                render_section(buffer, rect, label, *count, *collapsed, palette);
                hits.card_sections.push((rect, key.clone()));
            }
            BuiltItem::Section(_) | BuiltItem::Divider => {}
            BuiltItem::Card(card) => render_card(buffer, rect, snapshot, card, config, state, hits),
        }
        y = y.saturating_add(heights[position] + gaps[position]);
    }

    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.workspace_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, palette);
    }

    if let Some(row) = state
        .workspace_drop_indicator_row
        .filter(|row| *row >= body.y.saturating_sub(1) && *row < body.bottom())
    {
        put_text(
            buffer,
            body.x,
            row,
            width,
            &"─".repeat(width as usize),
            Style::default().fg(palette.accent),
        );
    }
}

fn render_section(
    buffer: &mut Buffer,
    rect: Rect,
    label: &str,
    count: usize,
    collapsed: bool,
    palette: &Palette,
) {
    let x = put_segment(
        buffer,
        rect.x.saturating_add(1),
        rect.y,
        rect.right(),
        if collapsed { "▸ " } else { "▾ " },
        Style::default().fg(palette.overlay0),
    );
    draw_line(
        buffer,
        Rect::new(x, rect.y, rect.right().saturating_sub(x), 1),
        &[(
            label.to_owned(),
            Style::default()
                .fg(palette.subtext0)
                .add_modifier(Modifier::BOLD),
        )],
        &[(count.to_string(), Style::default().fg(palette.overlay0))],
    );
}

fn render_card(
    buffer: &mut Buffer,
    rect: Rect,
    snapshot: &ClientShellSnapshot,
    card: &BuiltCard,
    config: &ClientShellConfig,
    state: &ShellRenderState<'_>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    let Some(workspace) = snapshot.workspaces.get(card.index) else {
        return;
    };
    let fill = |buffer: &mut Buffer, area: Rect, color: ratatui::style::Color| {
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                buffer[(x, y)].set_bg(color);
            }
        }
    };
    let selected = state
        .selected_workspace_id
        .is_some_and(|target| target.matches(state.active_endpoint_id, &workspace.workspace_id));
    // The frame needs room for its corners and one column inside.
    let framed = card.framed && rect.width >= 6;
    // The frame's right side; text stops one column before it.
    let inner_right = rect.right().saturating_sub(2);
    let first_line = rect.y.saturating_add(u16::from(framed));
    if selected {
        fill(buffer, rect, selection_background(palette));
    } else if framed {
        fill(
            buffer,
            Rect::new(
                rect.x + 2,
                first_line,
                inner_right.saturating_sub(rect.x + 2),
                rect.bottom().saturating_sub(first_line),
            ),
            super::sidebar::workspace_active_background(
                palette,
                state.selected_workspace_id.is_some(),
            ),
        );
    }

    let frame_style = Style::default().fg(palette.surface1);
    let frame_x = rect.x + FRAME_COLUMN;
    let frame_width = inner_right.saturating_sub(frame_x) + 1;
    let horizontal = "─".repeat(usize::from(frame_width.saturating_sub(2)));
    if framed {
        put_text(
            buffer,
            frame_x,
            rect.y,
            frame_width,
            &format!("╭{horizontal}╮"),
            frame_style,
        );
    }
    let mut y = first_line;
    for line in &card.lines {
        if y >= rect.bottom() {
            break;
        }
        let row = Rect::new(rect.x, y, rect.width, 1);
        if framed {
            put_text(buffer, frame_x, y, 1, "│", frame_style);
            put_text(buffer, inner_right, y, 1, "│", frame_style);
        }
        if line.highlight {
            let from = rect.x + 2;
            fill(
                buffer,
                Rect::new(from, y, inner_right.saturating_sub(from), 1),
                selection_background(palette),
            );
            put_text(buffer, from, y, 1, "▎", Style::default().fg(palette.accent));
        }
        for (offset, (text, style)) in &line.marks {
            let x = rect.x.saturating_add(*offset);
            if x < inner_right {
                put_text(buffer, x, y, 1, text, *style);
            }
        }
        let text_x = rect.x.saturating_add(line.indent);
        draw_line(
            buffer,
            Rect::new(text_x, y, inner_right.saturating_sub(text_x), 1),
            &line.left,
            &line.right,
        );
        if let Some(pane_id) = &line.pane_id {
            hits.card_sessions.push((row, pane_id.clone()));
        }
        if line.toggle {
            hits.card_session_toggles
                .push((row, workspace.workspace_id.clone()));
        }
        y += 1;
    }
    if framed && y < rect.bottom() {
        put_text(
            buffer,
            frame_x,
            y,
            frame_width,
            &format!("╰{horizontal}╯"),
            frame_style,
        );
    }
    // The whole card selects its workspace; session rows are checked first.
    hits.workspaces.push(WorkspaceHit {
        rect,
        endpoint_id: ClientEndpointId::Local,
        workspace_id: workspace.workspace_id.clone(),
        indented: false,
        group_toggle: None,
    });
}

impl ClientShellState {
    /// The card layout only replaces the expanded single-machine sidebar; the
    /// collapsed rail, mobile layout and multi-machine sidebar stay as they are.
    pub(super) fn cards_layout_active(&self) -> bool {
        self.config.sidebar_layout == crate::config::SidebarLayoutConfig::Cards
            && self.endpoints.len() == 1
            && !self.sidebar_collapsed
            && !self.mobile_layout_active()
    }

    fn cards_view(&self) -> CardsView<'_> {
        CardsView {
            group_by: self.config.cards.group_by,
            collapsed_groups: &self.collapsed_groups,
            pinned: &self.pinned_workspaces,
            filter: self.sidebar_filter.as_str(),
        }
    }

    pub(super) fn card_navigation_entries(
        &self,
        snapshot: &ClientShellSnapshot,
    ) -> Vec<WorkspaceEntry> {
        card_workspace_order(snapshot, &self.config.cards, &self.cards_view())
            .into_iter()
            .map(|index| WorkspaceEntry {
                index,
                indented: false,
                last_child: false,
            })
            .collect()
    }

    pub(super) fn reveal_card_workspace(&mut self, workspace_id: &str) {
        let target = self.snapshot.as_deref().and_then(|snapshot| {
            card_items(snapshot, &self.config.cards, &self.cards_view())
                .iter()
                .position(|item| {
                    matches!(item, CardItem::Card { index }
                        if snapshot.workspaces[*index].workspace_id == workspace_id)
                })
        });
        if let Some(target) = target {
            self.workspace_scroll = target.min(self.hits.workspace_max_scroll);
        }
    }

    /// Cards follow the workspace order, so dragging one reorders workspaces.
    /// Pinned cards, repository sections and a filtered list have their own
    /// order, so dragging is off there.
    pub(super) fn card_is_draggable(&self, workspace_id: &str) -> bool {
        self.config.cards.group_by == CardsGroupBy::None
            && self.sidebar_filter.trim().is_empty()
            && !self.pinned_workspaces.iter().any(|id| id == workspace_id)
    }

    /// Drop slots sit on the row above each unpinned top-level card, plus one
    /// below the last card when the list is not scrolled past it.
    pub(super) fn card_drop_target_at(&self, point: (u16, u16)) -> Option<(Option<String>, u16)> {
        let body = self.hits.workspace_body;
        if body.height == 0 || point.1 < body.y.saturating_sub(1) || point.1 >= body.bottom() {
            return None;
        }
        let snapshot = self.snapshot.as_deref()?;
        let is_root = |workspace_id: &str| {
            snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.workspace_id == workspace_id)
                .is_some_and(|workspace| {
                    !workspace
                        .worktree
                        .as_ref()
                        .is_some_and(|worktree| worktree.is_linked_worktree)
                })
        };
        let unpinned = self
            .hits
            .workspaces
            .iter()
            .filter(|hit| !self.pinned_workspaces.contains(&hit.workspace_id))
            .collect::<Vec<_>>();
        let mut slots = unpinned
            .iter()
            .filter(|hit| is_root(&hit.workspace_id))
            .map(|hit| (Some(hit.workspace_id.clone()), hit.rect.y.saturating_sub(1)))
            .collect::<Vec<_>>();
        let last = unpinned.last()?;
        let last_in_order = card_workspace_order(snapshot, &self.config.cards, &self.cards_view())
            .last()
            .map(|index| snapshot.workspaces[*index].workspace_id.as_str());
        if last_in_order == Some(last.workspace_id.as_str()) && last.rect.bottom() < body.bottom() {
            slots.push((None, last.rect.bottom()));
        }
        slots
            .into_iter()
            .enumerate()
            .min_by_key(|(index, (_, row))| (point.1.abs_diff(*row), *index))
            .map(|(_, target)| target)
    }

    pub(super) fn toggle_pinned_workspace(&mut self, workspace_id: String) {
        if let Some(position) = self
            .pinned_workspaces
            .iter()
            .position(|pinned| *pinned == workspace_id)
        {
            self.pinned_workspaces.remove(position);
        } else {
            self.pinned_workspaces.push(workspace_id);
        }
        // Forget pins whose workspace has since closed.
        if let Some(snapshot) = self.snapshot.as_deref() {
            self.pinned_workspaces.retain(|pinned| {
                snapshot
                    .workspaces
                    .iter()
                    .any(|workspace| workspace.workspace_id == *pinned)
            });
        }
    }

    pub(super) fn begin_sidebar_filter(&mut self) {
        if self.cards_layout_active()
            && matches!(self.overlay, None | Some(ClientShellOverlay::SidebarFilter))
        {
            self.overlay = Some(ClientShellOverlay::SidebarFilter);
        }
    }

    pub(super) fn insert_sidebar_filter_text(&mut self, text: &str) {
        if self.sidebar_filter.insert(text) {
            self.workspace_scroll = 0;
        }
    }

    /// Enter keeps the filter, Esc clears it; both stop editing.
    pub(super) fn route_sidebar_filter_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        use crossterm::event::KeyModifiers;

        outcome.repaint = true;
        match key.code {
            KeyCode::Enter => self.overlay = None,
            KeyCode::Esc => {
                self.overlay = None;
                self.sidebar_filter.clear();
                self.workspace_scroll = 0;
                self.reveal_focused_workspace = true;
            }
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.sidebar_filter.clear();
                self.workspace_scroll = 0;
            }
            _ => {
                if self.sidebar_filter.handle_key(key) == Some(true) {
                    self.workspace_scroll = 0;
                }
            }
        }
    }

    /// The next agent in `status` after the focused one, in card order.
    pub(super) fn next_card_agent(&self, status: AgentStatus) -> Option<String> {
        let snapshot = self.snapshot.as_deref()?;
        let order = card_workspace_order(snapshot, &self.config.cards, &self.cards_view());
        let candidates = order
            .iter()
            .flat_map(|index| {
                let workspace_id = &snapshot.workspaces[*index].workspace_id;
                snapshot.agents.iter().filter(move |agent| {
                    agent.workspace_id == *workspace_id && agent.agent_status == status
                })
            })
            .collect::<Vec<_>>();
        let current = candidates.iter().position(|agent| agent.focused);
        let next = current.map_or(0, |position| (position + 1) % candidates.len().max(1));
        candidates.get(next).map(|agent| agent.pane_id.clone())
    }

    /// Clicks on the card sidebar's own controls: grouping, filter, summary,
    /// section headers, session counters and session rows. Cards themselves use the shared
    /// workspace hits.
    pub(super) fn handle_card_sidebar_click(
        &mut self,
        point: (u16, u16),
        outcome: &mut ClientShellInput,
    ) -> bool {
        if super::contains(self.hits.card_group_toggle, point) {
            self.config.cards.group_by = self.config.cards.group_by.next();
            self.cards_group_by_manual = true;
            self.workspace_scroll = 0;
            self.reveal_focused_workspace = true;
            self.persist_chrome_preferences(outcome);
            outcome.repaint = true;
            return true;
        }
        if super::contains(self.hits.card_filter_toggle, point)
            || super::contains(self.hits.card_filter, point)
        {
            self.begin_sidebar_filter();
            outcome.repaint = true;
            return true;
        }
        if let Some(status) = self
            .hits
            .card_status
            .iter()
            .find(|(rect, _)| super::contains(*rect, point))
            .map(|(_, status)| *status)
        {
            if let Some(pane_id) = self.next_card_agent(status) {
                self.push_endpoint_method(
                    crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
                        pane_id,
                    }),
                    outcome,
                );
            }
            return true;
        }
        if let Some(key) = self
            .hits
            .card_sections
            .iter()
            .find(|(rect, _)| super::contains(*rect, point))
            .map(|(_, key)| key.clone())
        {
            self.toggle_collapsed_group(&ClientEndpointId::Local, key);
            self.persist_chrome_preferences(outcome);
            outcome.repaint = true;
            return true;
        }
        if let Some(workspace_id) = self
            .hits
            .card_session_toggles
            .iter()
            .find(|(rect, _)| super::contains(*rect, point))
            .map(|(_, workspace_id)| workspace_id.clone())
        {
            self.toggle_card_sessions(&workspace_id);
            self.persist_chrome_preferences(outcome);
            outcome.repaint = true;
            return true;
        }
        if let Some(pane_id) = self.card_session_at(point) {
            self.push_endpoint_method(
                crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget { pane_id }),
                outcome,
            );
            return true;
        }
        false
    }

    /// Folds or unfolds a card's session list, forgetting the folds of
    /// workspaces that have since closed.
    fn toggle_card_sessions(&mut self, workspace_id: &str) {
        self.toggle_collapsed_group(
            &ClientEndpointId::Local,
            format!("{SESSIONS_KEY_PREFIX}{workspace_id}"),
        );
        if let Some(snapshot) = self.snapshot.as_deref() {
            self.collapsed_groups.retain(|key| {
                key.strip_prefix(SESSIONS_KEY_PREFIX).is_none_or(|id| {
                    snapshot
                        .workspaces
                        .iter()
                        .any(|workspace| workspace.workspace_id == id)
                })
            });
        }
    }

    /// The pane of the session row under `point`.
    pub(super) fn card_session_at(&self, point: (u16, u16)) -> Option<String> {
        self.hits
            .card_sessions
            .iter()
            .find(|(rect, _)| super::contains(*rect, point))
            .map(|(_, pane_id)| pane_id.clone())
    }

    /// Runs the footer command, redraws card ages once a minute and saves
    /// agent state stamps when they are due.
    pub(crate) fn tick_cards_footer(&mut self, now: std::time::Instant) -> bool {
        let cards = self.config.sidebar_layout == crate::config::SidebarLayoutConfig::Cards;
        let mut changed = self.cards_footer.tick(
            now,
            cards,
            &self.config.cards.footer_command,
            self.config.cards.footer_interval_seconds,
        );
        if cards {
            changed |=
                self.cards_clock.minute_changed(SystemTime::now()) && self.cards_layout_active();
            if self.cards_clock.take_due_persist(now) {
                let mut outcome = ClientShellInput::default();
                self.persist_chrome_preferences(&mut outcome);
                changed |= outcome.repaint;
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_under_home_use_a_tilde() {
        assert_eq!(display_path("/home/me/notes", Some("/home/me")), "~/notes");
        assert_eq!(display_path("/home/me", Some("/home/me")), "~");
        assert_eq!(display_path("/home/meow", Some("/home/me")), "/home/meow");
        assert_eq!(display_path("/srv", None), "/srv");
    }
}
