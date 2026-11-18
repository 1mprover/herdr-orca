//! The session rows of each workspace card: its agents, then its terminals
//! (panes without an agent), in tab and pane order. The order never depends on
//! agent state, so a row stays where it is when its agent changes state. While
//! the sidebar is filtered, matching rows come first.

use ratatui::style::Color;

use super::*;
use crate::config::CardsSidebarConfig;
use crate::protocol::{ClientShellAgent, ClientShellPane};

pub(super) enum SessionKind<'a> {
    Agent(&'a ClientShellAgent),
    Terminal {
        tab_label: &'a str,
        pane_label: String,
        tab_has_one_pane: bool,
    },
}

pub(super) struct SessionRow<'a> {
    pub(super) pane_id: &'a str,
    pub(super) kind: SessionKind<'a>,
    /// The pane the client is focused on.
    pub(super) focused: bool,
}

#[derive(Default)]
pub(super) struct WorkspaceSessions<'a> {
    pub(super) agents: usize,
    pub(super) terminals: usize,
    pub(super) rows: Vec<SessionRow<'a>>,
}

/// Every workspace's rows, built in one pass over the snapshot per render.
pub(super) struct SessionIndex<'a> {
    workspaces: HashMap<&'a str, WorkspaceSessions<'a>>,
    empty: WorkspaceSessions<'a>,
}

struct Pending<'a> {
    workspace_id: &'a str,
    order: (usize, usize),
    matches: bool,
    row: SessionRow<'a>,
}

impl<'a> SessionIndex<'a> {
    /// `needle` is the trimmed, lowercased filter; empty means no filter.
    pub(super) fn new(snapshot: &'a ClientShellSnapshot, needle: &str) -> Self {
        let tabs = snapshot
            .tabs
            .iter()
            .enumerate()
            .map(|(position, tab)| (tab.tab_id.as_str(), (position, tab.label.as_str())))
            .collect::<HashMap<_, _>>();
        let mut panes_per_tab = HashMap::<&str, usize>::new();
        for pane in &snapshot.panes {
            *panes_per_tab.entry(pane.tab_id.as_str()).or_default() += 1;
        }
        let panes = snapshot
            .panes
            .iter()
            .enumerate()
            .map(|(position, pane)| (pane.pane_id.as_str(), (position, pane)))
            .collect::<HashMap<_, _>>();
        let focused_pane = snapshot.focused_pane_id.as_deref();
        let filtering = !needle.is_empty();
        let hit = |text: &str| filtering && text.to_lowercase().contains(needle);
        let tab_of = |tab_id: &str| tabs.get(tab_id).copied().unwrap_or((usize::MAX, ""));

        let mut agent_rows = Vec::new();
        for agent in &snapshot.agents {
            let (tab_position, tab_label) = tab_of(&agent.tab_id);
            let pane = panes.get(agent.pane_id.as_str());
            let pane_position = pane.map_or(usize::MAX, |(position, _)| *position);
            let matches = filtering
                && (agent_matches(agent, needle)
                    || hit(tab_label)
                    || pane.is_some_and(|(_, pane)| hit(&pane_label(pane))));
            agent_rows.push(Pending {
                workspace_id: &agent.workspace_id,
                order: (tab_position, pane_position),
                matches,
                row: SessionRow {
                    pane_id: &agent.pane_id,
                    kind: SessionKind::Agent(agent),
                    focused: focused_pane == Some(agent.pane_id.as_str()),
                },
            });
        }
        let with_agent = snapshot
            .agents
            .iter()
            .map(|agent| agent.pane_id.as_str())
            .collect::<HashSet<_>>();
        let mut terminal_rows = Vec::new();
        for (pane_position, pane) in snapshot.panes.iter().enumerate() {
            if with_agent.contains(pane.pane_id.as_str()) {
                continue;
            }
            let (tab_position, tab_label) = tab_of(&pane.tab_id);
            let label = pane_label(pane);
            let matches = hit(tab_label) || hit(&label);
            terminal_rows.push(Pending {
                workspace_id: &pane.workspace_id,
                order: (tab_position, pane_position),
                matches,
                row: SessionRow {
                    pane_id: &pane.pane_id,
                    kind: SessionKind::Terminal {
                        tab_label,
                        pane_label: label,
                        tab_has_one_pane: panes_per_tab.get(pane.tab_id.as_str()) == Some(&1),
                    },
                    focused: focused_pane == Some(pane.pane_id.as_str()),
                },
            });
        }
        agent_rows.sort_by_key(|pending| pending.order);
        terminal_rows.sort_by_key(|pending| pending.order);

        let mut workspaces = HashMap::<&str, WorkspaceSessions<'a>>::new();
        let mut matched = HashMap::<&str, usize>::new();
        for (is_agent, pending) in agent_rows
            .into_iter()
            .map(|pending| (true, pending))
            .chain(terminal_rows.into_iter().map(|pending| (false, pending)))
        {
            let sessions = workspaces.entry(pending.workspace_id).or_default();
            if is_agent {
                sessions.agents += 1;
            } else {
                sessions.terminals += 1;
            }
            // Matching rows go before the unmatched ones, keeping their order.
            if pending.matches {
                let slot = matched.entry(pending.workspace_id).or_default();
                sessions.rows.insert(*slot, pending.row);
                *slot += 1;
            } else {
                sessions.rows.push(pending.row);
            }
        }
        Self {
            workspaces,
            empty: WorkspaceSessions::default(),
        }
    }

    /// The rows of one workspace; none for an unknown workspace.
    pub(super) fn sessions(&self, workspace_id: &str) -> &WorkspaceSessions<'a> {
        self.workspaces.get(workspace_id).unwrap_or(&self.empty)
    }
}

/// Which rows of a card to show: `len` rows from `start`, with `hidden` more
/// summed up on one extra line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RowWindow {
    pub(super) start: usize,
    pub(super) len: usize,
    pub(super) hidden: usize,
}

/// Fits `total` rows into `limit` rows plus a "… N more" line. A single
/// hidden row is shown instead, since its line costs the same. The window
/// slides to keep the `focused` row in view.
pub(super) fn row_window(total: usize, limit: usize, focused: Option<usize>) -> RowWindow {
    if limit == 0 {
        return RowWindow {
            start: 0,
            len: 0,
            hidden: 0,
        };
    }
    if total <= limit + 1 {
        return RowWindow {
            start: 0,
            len: total,
            hidden: 0,
        };
    }
    let start = match focused {
        Some(focused) if focused >= limit => (focused + 1 - limit).min(total - limit),
        _ => 0,
    };
    RowWindow {
        start,
        len: limit,
        hidden: total - limit,
    }
}

pub(super) fn agent_matches(agent: &ClientShellAgent, needle: &str) -> bool {
    [
        &agent.display_agent,
        &agent.name,
        &agent.agent,
        &agent.title,
        &agent.terminal_title_stripped,
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(needle))
}

fn basename(path: &str) -> Option<&str> {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
}

pub(super) fn pane_label(pane: &ClientShellPane) -> String {
    pane.label
        .as_deref()
        .filter(|label| !label.is_empty())
        .or_else(|| pane.foreground_cwd.as_deref().and_then(basename))
        .or_else(|| pane.cwd.as_deref().and_then(basename))
        .unwrap_or("terminal")
        .to_owned()
}

/// A mark for the kind of agent, drawn from glyphs plain monospace fonts have.
pub(super) fn agent_glyph(agent: &ClientShellAgent, palette: &Palette) -> (&'static str, Color) {
    let kind = agent
        .agent
        .as_deref()
        .or(agent.display_agent.as_deref())
        .or(agent.name.as_deref())
        .and_then(|kind| kind.split_whitespace().next())
        .unwrap_or_default()
        .to_lowercase();
    match kind.as_str() {
        "claude" => ("✳", palette.peach),
        "codex" => ("◎", palette.subtext0),
        "gemini" => ("✦", palette.blue),
        "opencode" => ("■", palette.text),
        "cursor" => ("▲", palette.text),
        "github-copilot" => ("◈", palette.mauve),
        _ => ("◆", palette.overlay1),
    }
}

/// `"{count} {word}"` with the word from the `{key}_one`, `{key}_few` or
/// `{key}_many` label. The one/few/many split follows Russian and also covers
/// English, whose `few` and `many` are the same word.
pub(super) fn count_label(
    cards: &CardsSidebarConfig,
    key: &str,
    count: usize,
    fallback: [&'static str; 3],
) -> String {
    let form = if count % 10 == 1 && count % 100 != 11 {
        0
    } else if (2..=4).contains(&(count % 10)) && !(12..=14).contains(&(count % 100)) {
        1
    } else {
        2
    };
    let suffix = ["one", "few", "many"][form];
    let word = cards.label(&format!("{key}_{suffix}"), fallback[form]);
    format!("{count} {word}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basename_ignores_trailing_slashes() {
        assert_eq!(basename("/home/me/project/"), Some("project"));
        assert_eq!(basename("/"), None);
    }
}
