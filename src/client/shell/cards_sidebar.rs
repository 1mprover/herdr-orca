//! The card sidebar: one list of workspace cards with their tabs and panes
//! nested inside, sorted into collapsible sections (pinned, then by status or
//! project), with a filter field on top and an optional status line below.

use super::*;
use crate::api::schema::AgentStatus;
use crate::config::{AgentsSidebarConfig, CardsGroupBy, CardsSidebarConfig};
use crate::ui::{ResolvedToken, ResolvedTokenKind};
use ratatui::{
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use super::render::{
    display_width, put_right_text, put_segment, put_text, render_sidebar_background,
    ShellRenderState,
};

/// Section collapse state shares `collapsed_groups` with worktree groups, so
/// card sections carry a prefix that no repository key can start with.
pub(super) const SECTION_KEY_PREFIX: &str = "cards:";
const HEADER_ROWS: u16 = 2;

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
        status: Option<AgentStatus>,
        count: usize,
        collapsed: bool,
    },
    Card {
        index: usize,
    },
}

fn section_label(cards: &CardsSidebarConfig, id: &str, fallback: &str) -> String {
    cards
        .section_labels
        .get(id)
        .filter(|label| !label.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| fallback.to_owned())
}

fn status_section(status: AgentStatus) -> (&'static str, &'static str) {
    match status {
        AgentStatus::Blocked => ("blocked", "needs input"),
        AgentStatus::Done => ("done", "done"),
        AgentStatus::Working => ("working", "working"),
        AgentStatus::Idle | AgentStatus::Unknown => ("idle", "idle"),
    }
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
            .any(|pane| pane.workspace_id == id && pane.label.as_deref().is_some_and(hit))
        || snapshot
            .agents
            .iter()
            .filter(|agent| agent.workspace_id == id)
            .any(|agent| {
                [
                    &agent.display_agent,
                    &agent.name,
                    &agent.agent,
                    &agent.title,
                    &agent.terminal_title_stripped,
                ]
                .into_iter()
                .flatten()
                .any(|value| hit(value))
            })
}

/// Sections and cards in display order. Collapsed sections keep the focused
/// card visible; a non-empty filter shows every match regardless of collapse.
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
    let rest = (0..snapshot.workspaces.len())
        .filter(|index| {
            !view
                .pinned
                .contains(&snapshot.workspaces[*index].workspace_id)
                && visible(*index)
        })
        .collect::<Vec<_>>();

    let mut groups: Vec<(String, String, Option<AgentStatus>, Vec<usize>)> = Vec::new();
    if !pinned.is_empty() {
        groups.push((
            "pinned".to_owned(),
            section_label(cards, "pinned", "pinned"),
            None,
            pinned,
        ));
    }
    match view.group_by {
        CardsGroupBy::Status => {
            for status in [
                AgentStatus::Blocked,
                AgentStatus::Done,
                AgentStatus::Working,
                AgentStatus::Idle,
            ] {
                let (id, fallback) = status_section(status);
                let members = rest
                    .iter()
                    .copied()
                    .filter(|index| {
                        status_section(snapshot.workspaces[*index].agent_status).0 == id
                    })
                    .collect();
                groups.push((
                    format!("status:{id}"),
                    section_label(cards, id, fallback),
                    Some(status),
                    members,
                ));
            }
        }
        CardsGroupBy::Project => {
            let mut projects: Vec<(String, String, Vec<usize>)> = Vec::new();
            for index in rest {
                let worktree = snapshot.workspaces[index].worktree.as_ref();
                let key = worktree.map_or("", |worktree| worktree.key.as_str());
                if let Some(project) = projects.iter_mut().find(|project| project.0 == key) {
                    project.2.push(index);
                } else {
                    let label = worktree.map_or_else(
                        || section_label(cards, "no_repo", "no repo"),
                        |worktree| worktree.label.clone(),
                    );
                    projects.push((key.to_owned(), label, vec![index]));
                }
            }
            // Workspaces outside any repository go last.
            projects.sort_by_key(|project| project.0.is_empty());
            for (key, label, members) in projects {
                groups.push((format!("project:{key}"), label, None, members));
            }
        }
        CardsGroupBy::None => {
            if groups.is_empty() {
                return rest
                    .into_iter()
                    .map(|index| CardItem::Card { index })
                    .collect();
            }
            groups.push((
                "rest".to_owned(),
                section_label(cards, "rest", "others"),
                None,
                rest,
            ));
        }
    }

    let mut items = Vec::new();
    for (id, label, status, members) in groups {
        if members.is_empty() {
            continue;
        }
        let key = format!("{SECTION_KEY_PREFIX}{id}");
        let collapsed = !filtering && view.collapsed_groups.contains(&key);
        items.push(CardItem::Section {
            key,
            label,
            status,
            count: members.len(),
            collapsed,
        });
        for index in members {
            if !collapsed || snapshot.workspaces[index].focused {
                items.push(CardItem::Card { index });
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
            CardItem::Section { .. } => None,
        })
        .collect()
}

enum LineTarget {
    Workspace,
    Tab(String),
    Pane(String),
}

enum LineBody {
    Space(Vec<ResolvedToken>),
    Agent {
        tokens: Vec<ResolvedToken>,
        status: AgentStatus,
        focused: bool,
    },
    Tab {
        label: String,
        focused: bool,
    },
    Pane {
        label: String,
        focused: bool,
    },
}

struct CardLine {
    prefix: String,
    body: LineBody,
    target: LineTarget,
}

fn cards_agents_config(cards: &CardsSidebarConfig) -> AgentsSidebarConfig {
    AgentsSidebarConfig {
        rows: cards.agent_rows.clone(),
        rows_by_agent: Default::default(),
        row_gap: 0,
    }
}

fn basename(path: &str) -> Option<&str> {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
}

fn pane_label(pane: &crate::protocol::ClientShellPane) -> String {
    pane.label
        .as_deref()
        .filter(|label| !label.is_empty())
        .or_else(|| pane.foreground_cwd.as_deref().and_then(basename))
        .or_else(|| pane.cwd.as_deref().and_then(basename))
        .unwrap_or("terminal")
        .to_owned()
}

/// Drops rows that would print nothing, such as an agent with no title yet.
fn non_empty_rows(rows: Vec<Vec<ResolvedToken>>) -> Vec<Vec<ResolvedToken>> {
    rows.into_iter()
        .map(|row| {
            row.into_iter()
                .filter(|token| match &token.kind {
                    ResolvedTokenKind::TerminalTitle(value)
                    | ResolvedTokenKind::Pane(value)
                    | ResolvedTokenKind::Custom(value) => !value.trim().is_empty(),
                    _ => true,
                })
                .collect::<Vec<_>>()
        })
        .filter(|row| !row.is_empty())
        .collect()
}

fn card_lines(
    snapshot: &ClientShellSnapshot,
    index: usize,
    config: &ClientShellConfig,
    agents: &AgentsSidebarConfig,
) -> Vec<CardLine> {
    let Some(workspace) = snapshot.workspaces.get(index) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for (row_index, row) in
        super::sidebar::workspace_rows(workspace, workspace.agent_status, false, &config.spaces)
            .into_iter()
            .enumerate()
    {
        let prefix = if row_index == 0 {
            ""
        } else if matches!(
            row.first().map(|token| &token.kind),
            Some(ResolvedTokenKind::Branch(_))
        ) {
            "  ⎇ "
        } else {
            "  "
        };
        lines.push(CardLine {
            prefix: prefix.to_owned(),
            body: LineBody::Space(row),
            target: LineTarget::Workspace,
        });
    }
    if lines.is_empty() {
        lines.push(CardLine {
            prefix: String::new(),
            body: LineBody::Space(vec![
                ResolvedToken {
                    kind: ResolvedTokenKind::StateIcon,
                    style: Default::default(),
                },
                ResolvedToken {
                    kind: ResolvedTokenKind::Workspace(workspace.label.clone()),
                    style: Default::default(),
                },
            ]),
            target: LineTarget::Workspace,
        });
    }

    let id = workspace.workspace_id.as_str();
    let tabs = snapshot
        .tabs
        .iter()
        .filter(|tab| tab.workspace_id == id)
        .collect::<Vec<_>>();
    let show_tabs = tabs.len() > 1 || tabs.iter().any(|tab| tab.custom_label);
    for (tab_position, tab) in tabs.iter().enumerate() {
        let last_tab = tab_position + 1 == tabs.len();
        let panes = snapshot
            .panes
            .iter()
            .filter(|pane| pane.tab_id == tab.tab_id)
            .collect::<Vec<_>>();
        let is_agent = |pane_id: &str| snapshot.agents.iter().any(|agent| agent.pane_id == pane_id);
        // A lone shell adds nothing the tab line does not already say.
        let shown = if panes.len() > 1 {
            panes
        } else {
            panes
                .into_iter()
                .filter(|pane| is_agent(pane.pane_id.as_str()))
                .collect()
        };
        let child_base = if show_tabs {
            lines.push(CardLine {
                prefix: format!("  {} ", if last_tab { "└" } else { "├" }),
                body: LineBody::Tab {
                    label: tab.label.clone(),
                    focused: tab.focused,
                },
                target: LineTarget::Tab(tab.tab_id.clone()),
            });
            if last_tab {
                "     "
            } else {
                "  │  "
            }
        } else {
            "  "
        };
        for (pane_position, pane) in shown.iter().enumerate() {
            let last_pane = pane_position + 1 == shown.len();
            let branch = format!("{child_base}{} ", if last_pane { "└" } else { "├" });
            let continuation = format!("{child_base}{}  ", if last_pane { "  " } else { "│ " });
            if let Some(row) =
                super::agent_sidebar::agent_row_with(snapshot, &pane.pane_id, agents, None)
            {
                let mut rows = non_empty_rows(row.rows);
                if rows.is_empty() {
                    rows.push(vec![ResolvedToken {
                        kind: ResolvedTokenKind::StateIcon,
                        style: Default::default(),
                    }]);
                }
                for (row_index, tokens) in rows.into_iter().enumerate() {
                    lines.push(CardLine {
                        prefix: if row_index == 0 {
                            branch.clone()
                        } else {
                            continuation.clone()
                        },
                        body: LineBody::Agent {
                            tokens,
                            status: row.status,
                            focused: row.focused,
                        },
                        target: LineTarget::Pane(pane.pane_id.clone()),
                    });
                }
            } else {
                lines.push(CardLine {
                    prefix: branch,
                    body: LineBody::Pane {
                        label: pane_label(pane),
                        focused: pane.focused,
                    },
                    target: LineTarget::Pane(pane.pane_id.clone()),
                });
            }
        }
    }
    lines
}

enum BuiltItem {
    Section(CardItem),
    Card(usize, Vec<CardLine>),
}

impl BuiltItem {
    fn height(&self) -> u16 {
        match self {
            Self::Section(_) => 1,
            Self::Card(_, lines) => lines.len().clamp(1, u16::MAX as usize) as u16,
        }
    }
}

fn selection_background(palette: &Palette) -> ratatui::style::Color {
    if palette.selection_bg == ratatui::style::Color::Reset {
        palette.active_row_bg
    } else {
        palette.selection_bg
    }
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

    let header_style = Style::default()
        .fg(palette.overlay0)
        .add_modifier(Modifier::BOLD);
    put_text(
        buffer,
        content.x,
        content.y,
        content.width,
        " workspaces",
        header_style,
    );
    let group_label = config.cards.group_by.label();
    let group_width = display_width(group_label).min(content.width.saturating_sub(1));
    let group_rect = Rect::new(
        content.right().saturating_sub(group_width + 1),
        content.y,
        group_width,
        1,
    );
    put_text(
        buffer,
        group_rect.x,
        group_rect.y,
        group_rect.width,
        group_label,
        header_style,
    );
    hits.card_group_toggle = if config.mouse_capture {
        group_rect
    } else {
        Rect::default()
    };

    if content.height > 1 {
        let y = content.y + 1;
        hits.card_filter = Rect::new(content.x, y, content.width, 1);
        let filtering = !state.sidebar_filter.trim().is_empty();
        put_text(
            buffer,
            content.x.saturating_add(1),
            y,
            content.width.saturating_sub(1),
            "⌕",
            Style::default().fg(if state.sidebar_filter_editing || filtering {
                palette.accent
            } else {
                palette.overlay0
            }),
        );
        let field = Rect::new(
            content.x.saturating_add(3),
            y,
            content.width.saturating_sub(4),
            1,
        );
        if state.sidebar_filter_editing {
            hits.card_filter_cursor = super::text_editor::render(
                buffer,
                field,
                state.sidebar_filter,
                Style::default().fg(palette.text).bg(palette.surface0),
            );
        } else if filtering {
            put_text(
                buffer,
                field.x,
                y,
                field.width,
                state.sidebar_filter.as_str(),
                Style::default().fg(palette.accent),
            );
        } else {
            put_text(
                buffer,
                field.x,
                y,
                field.width,
                "filter",
                Style::default()
                    .fg(palette.overlay0)
                    .add_modifier(Modifier::DIM),
            );
        }
    }

    let footer_y = content.bottom().saturating_sub(1);
    let status_line = state
        .cards_footer
        .filter(|text| !text.is_empty() && content.height > HEADER_ROWS + 3);
    let reserved = 1 + u16::from(status_line.is_some());
    let body = Rect::new(
        content.x,
        content.y.saturating_add(HEADER_ROWS),
        content.width,
        content.height.saturating_sub(HEADER_ROWS + reserved),
    );
    hits.workspace_body = body;

    let view = CardsView {
        group_by: config.cards.group_by,
        collapsed_groups: state.collapsed_groups,
        pinned: state.pinned_workspaces,
        filter: state.sidebar_filter.as_str(),
    };
    let agents = cards_agents_config(&config.cards);
    let items = card_items(snapshot, &config.cards, &view)
        .into_iter()
        .map(|item| match item {
            CardItem::Card { index } => {
                BuiltItem::Card(index, card_lines(snapshot, index, config, &agents))
            }
            section => BuiltItem::Section(section),
        })
        .collect::<Vec<_>>();
    render_card_list(buffer, body, snapshot, config, state, hits, &items);

    if let Some(text) = status_line {
        let y = footer_y.saturating_sub(1);
        put_text(
            buffer,
            content.x,
            y,
            content.width,
            &"─".repeat(content.width as usize),
            Style::default().fg(palette.surface_dim),
        );
        put_text(
            buffer,
            content.x.saturating_add(1),
            y,
            content.width.saturating_sub(2),
            &format!(" {text} "),
            Style::default().fg(palette.subtext0),
        );
    }

    if config.mouse_capture {
        hits.new_workspace = Rect::new(content.x, footer_y, 5.min(content.width), 1);
        put_text(
            buffer,
            content.x,
            footer_y,
            content.width,
            " new",
            Style::default().fg(palette.overlay0),
        );
        // Leave the last columns to the collapse toggle.
        let menu_area = Rect::new(content.x, footer_y, content.width.saturating_sub(2), 1);
        let attention = super::global_menu::global_menu_attention(snapshot);
        let launcher_width = if attention { 8 } else { 6 }.min(menu_area.width);
        hits.global_launcher = Rect::new(
            menu_area.right().saturating_sub(launcher_width),
            footer_y,
            launcher_width,
            1,
        );
        if attention {
            let start_x = menu_area.right().saturating_sub(6);
            put_text(
                buffer,
                start_x,
                footer_y,
                2,
                "● ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            );
            put_text(
                buffer,
                start_x.saturating_add(2),
                footer_y,
                4,
                "menu",
                Style::default().fg(palette.overlay0),
            );
        } else {
            put_right_text(
                buffer,
                menu_area,
                footer_y,
                "menu",
                Style::default().fg(palette.overlay0),
            );
        }
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
        Style::default().fg(palette.overlay0),
    );
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
        put_text(
            buffer,
            body.x,
            body.y,
            body.width,
            if state.sidebar_filter.trim().is_empty() {
                " no workspaces"
            } else {
                " no matches"
            },
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
            (BuiltItem::Card(..), Some(BuiltItem::Card(..))) => config.cards.card_gap,
            (BuiltItem::Card(..), Some(BuiltItem::Section(_))) => config.cards.card_gap.max(1),
            _ => 0,
        })
        .collect::<Vec<_>>();
    let mut metrics =
        super::scroll::list_scroll_metrics(&heights, &gaps, body.height, *state.workspace_scroll);
    if std::mem::take(state.reveal_focused_workspace) {
        if let Some(target) = items.iter().position(
            |item| matches!(item, BuiltItem::Card(index, _) if snapshot.workspaces[*index].focused),
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
                status,
                count,
                collapsed,
            }) => {
                render_section(
                    buffer, rect, label, *status, *count, *collapsed, config, palette,
                );
                hits.card_sections.push((rect, key.clone()));
            }
            BuiltItem::Section(CardItem::Card { .. }) => {}
            BuiltItem::Card(index, lines) => {
                render_card(buffer, rect, snapshot, *index, lines, config, state, hits);
            }
        }
        y = y.saturating_add(heights[position] + gaps[position]);
    }

    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.workspace_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, palette);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_section(
    buffer: &mut Buffer,
    rect: Rect,
    label: &str,
    status: Option<AgentStatus>,
    count: usize,
    collapsed: bool,
    config: &ClientShellConfig,
    palette: &Palette,
) {
    let mut x = rect.x.saturating_add(1);
    x = put_segment(
        buffer,
        x,
        rect.y,
        rect.right(),
        if collapsed { "▸ " } else { "▾ " },
        Style::default().fg(palette.overlay0),
    );
    let color = status.map_or(palette.overlay1, |status| status_color(status, palette));
    if let Some(status) = status {
        x = put_segment(
            buffer,
            x,
            rect.y,
            rect.right(),
            &format!("{} ", status_icon(status, config.status_indicators)),
            Style::default().fg(color),
        );
    }
    x = put_segment(
        buffer,
        x,
        rect.y,
        rect.right(),
        &label.to_uppercase(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    );
    put_segment(
        buffer,
        x,
        rect.y,
        rect.right(),
        &format!(" · {count}"),
        Style::default().fg(palette.overlay0),
    );
}

#[allow(clippy::too_many_arguments)]
fn render_card(
    buffer: &mut Buffer,
    rect: Rect,
    snapshot: &ClientShellSnapshot,
    index: usize,
    lines: &[CardLine],
    config: &ClientShellConfig,
    state: &ShellRenderState<'_>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    let Some(workspace) = snapshot.workspaces.get(index) else {
        return;
    };
    let status = workspace.agent_status;
    let focused = workspace.focused;
    let selected = state
        .selected_workspace_id
        .is_some_and(|target| target.matches(state.active_endpoint_id, &workspace.workspace_id));
    let bar_color = match status {
        AgentStatus::Blocked | AgentStatus::Working | AgentStatus::Done => {
            status_color(status, palette)
        }
        AgentStatus::Idle | AgentStatus::Unknown if focused => palette.accent,
        AgentStatus::Idle | AgentStatus::Unknown => palette.surface1,
    };
    let guide = Style::default().fg(palette.overlay0);
    let mut workspace_rows = 0u16;
    for (line_index, line) in lines.iter().take(rect.height as usize).enumerate() {
        let y = rect.y + line_index as u16;
        let line_rect = Rect::new(rect.x, y, rect.width, 1);
        put_text(buffer, rect.x, y, 1, "▎", Style::default().fg(bar_color));
        let x = put_segment(
            buffer,
            rect.x.saturating_add(1),
            y,
            rect.right(),
            &line.prefix,
            guide,
        );
        let available = rect.right().saturating_sub(x).saturating_sub(1) as usize;
        let spans: Vec<Span<'static>> = match &line.body {
            LineBody::Space(tokens) => crate::ui::resolved_token_spans(
                tokens,
                (
                    status_icon(status, config.status_indicators),
                    Style::default().fg(status_color(status, palette)),
                ),
                Style::default().fg(status_color(status, palette)),
                Style::default()
                    .fg(if focused {
                        palette.text
                    } else {
                        palette.subtext0
                    })
                    .add_modifier(Modifier::BOLD),
                Style::default().fg(if focused {
                    palette.mauve
                } else {
                    palette.overlay0
                }),
                Style::default().fg(palette.overlay1),
                palette,
                available,
            ),
            LineBody::Agent {
                tokens,
                status,
                focused,
            } => crate::ui::resolved_token_spans(
                tokens,
                (
                    status_icon(*status, config.status_indicators),
                    Style::default().fg(status_color(*status, palette)),
                ),
                Style::default().fg(status_color(*status, palette)),
                Style::default().fg(if *focused {
                    palette.text
                } else {
                    palette.subtext0
                }),
                Style::default().fg(palette.overlay0),
                Style::default().fg(palette.overlay0),
                palette,
                available,
            ),
            LineBody::Tab {
                label,
                focused: tab_focused,
            } => vec![Span::styled(
                label.clone(),
                if *tab_focused && focused {
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.subtext0)
                },
            )],
            LineBody::Pane {
                label,
                focused: pane_focused,
            } => vec![
                Span::styled("› ", guide),
                Span::styled(
                    label.clone(),
                    Style::default().fg(if *pane_focused {
                        palette.text
                    } else {
                        palette.overlay1
                    }),
                ),
            ],
        };
        Paragraph::new(Line::from(spans))
            .render(Rect::new(x, y, rect.right().saturating_sub(x), 1), buffer);
        match &line.target {
            LineTarget::Workspace => workspace_rows += 1,
            LineTarget::Tab(tab_id) => hits.card_tabs.push((line_rect, tab_id.clone())),
            LineTarget::Pane(pane_id) => hits.agents.push((line_rect, pane_id.clone())),
        }
    }
    if workspace_rows > 0 {
        hits.workspaces.push(WorkspaceHit {
            rect: Rect::new(rect.x, rect.y, rect.width, workspace_rows),
            endpoint_id: ClientEndpointId::Local,
            workspace_id: workspace.workspace_id.clone(),
            indented: false,
            group_toggle: None,
        });
    }

    let background = if selected {
        Some(selection_background(palette))
    } else if focused {
        Some(super::sidebar::workspace_active_background(
            palette,
            state.selected_workspace_id.is_some(),
        ))
    } else {
        None
    };
    if let Some(background) = background {
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                buffer[(x, y)].set_bg(background);
            }
        }
    }
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

    /// Clicks on the card sidebar's own controls: grouping, filter, section
    /// headers and tab lines. Workspace and pane lines use the shared hits.
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
        if super::contains(self.hits.card_filter, point) {
            self.begin_sidebar_filter();
            outcome.repaint = true;
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
        if let Some(tab_id) = self
            .hits
            .card_tabs
            .iter()
            .find(|(rect, _)| super::contains(*rect, point))
            .map(|(_, tab_id)| tab_id.clone())
        {
            self.push_endpoint_method(
                crate::api::schema::Method::TabFocus(crate::api::schema::TabTarget { tab_id }),
                outcome,
            );
            return true;
        }
        false
    }

    pub(crate) fn tick_cards_footer(&mut self, now: std::time::Instant) -> bool {
        self.cards_footer.tick(
            now,
            self.config.sidebar_layout == crate::config::SidebarLayoutConfig::Cards,
            &self.config.cards.footer_command,
            self.config.cards.footer_interval_seconds,
        )
    }
}

/// Runs `[ui.sidebar.cards].footer_command` off the input thread and keeps its
/// latest first line for the sidebar's status row.
#[derive(Default)]
pub(crate) struct CardsFooterRuntime {
    command: String,
    text: Option<String>,
    next_run_at: Option<std::time::Instant>,
    pending: Option<std::sync::mpsc::Receiver<Option<String>>>,
}

const FOOTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const FOOTER_MAX_CHARS: usize = 120;

impl CardsFooterRuntime {
    pub(super) fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Collects a finished run and starts the next one when due. Returns
    /// whether the visible text changed.
    pub(super) fn tick(
        &mut self,
        now: std::time::Instant,
        enabled: bool,
        command: &str,
        interval_seconds: u64,
    ) -> bool {
        let command = command.trim();
        if !enabled || command.is_empty() {
            self.pending = None;
            self.next_run_at = None;
            self.command.clear();
            return self.text.take().is_some();
        }
        let mut changed = false;
        if self.command != command {
            self.command = command.to_owned();
            self.pending = None;
            self.next_run_at = None;
            changed |= self.text.take().is_some();
        }
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(output) => {
                    self.pending = None;
                    changed |= self.text != output;
                    self.text = output;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.pending = None,
            }
        }
        if self.pending.is_none() && self.next_run_at.is_none_or(|deadline| now >= deadline) {
            let interval = std::time::Duration::from_secs(interval_seconds.clamp(1, 3600));
            self.next_run_at = Some(now + interval);
            self.pending = spawn_footer_command(self.command.clone());
        }
        changed
    }
}

#[cfg(unix)]
fn spawn_footer_command(command: String) -> Option<std::sync::mpsc::Receiver<Option<String>>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("herdr-cards-footer".to_owned())
        .spawn(move || {
            let _ = sender.send(run_footer_command(&command));
        })
        .ok()?;
    Some(receiver)
}

#[cfg(not(unix))]
fn spawn_footer_command(_command: String) -> Option<std::sync::mpsc::Receiver<Option<String>>> {
    None
}

#[cfg(unix)]
fn run_footer_command(command: &str) -> Option<String> {
    use std::io::Read as _;
    use std::process::{Command, Stdio};

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < FOOTER_TIMEOUT => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut output = String::new();
    child.stdout.take()?.read_to_string(&mut output).ok()?;
    footer_line(&output)
}

/// First non-empty output line with terminal escapes and control characters
/// removed, so a script written for a colored prompt still renders cleanly.
fn footer_line(output: &str) -> Option<String> {
    let line = output.lines().find(|line| !line.trim().is_empty())?;
    let mut clean = String::new();
    let mut chars = line.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        if !character.is_control() {
            clean.push(character);
        }
    }
    let clean = clean.trim();
    (!clean.is_empty()).then(|| clean.chars().take(FOOTER_MAX_CHARS).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_line_strips_escapes_and_keeps_the_first_line() {
        assert_eq!(
            footer_line("\n\u{1b}[32m5h 80%\u{1b}[0m · 7d 91%\nsecond\n").as_deref(),
            Some("5h 80% · 7d 91%")
        );
        assert_eq!(footer_line("  \n\t\n"), None);
    }

    #[test]
    fn basename_ignores_trailing_slashes() {
        assert_eq!(basename("/home/me/project/"), Some("project"));
        assert_eq!(basename("/"), None);
    }
}
