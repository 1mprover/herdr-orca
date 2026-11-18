//! How long each agent has been in its current state, for the card sidebar.
//!
//! The server only reports a per-pane state change sequence, so this client
//! stamps a change when it sees the sequence move. Stamps are saved with the
//! chrome preferences so a reattached client keeps them; agents seen for the
//! first time at startup without a saved stamp have no age until they change.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::preferences::AgentStateSince;
use crate::protocol::ClientShellAgent;

/// Saved stamps are written at most this often while agents keep changing.
const PERSIST_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
pub(crate) struct AgentActivityClock {
    boot_id: Option<String>,
    /// Pane id to its state change sequence and when that state began.
    entries: HashMap<String, (u64, Option<SystemTime>)>,
    /// Stamps loaded from preferences, consulted for the first snapshot only.
    seeds: Vec<AgentStateSince>,
    dirty: bool,
    last_persist: Option<Instant>,
    minute: Option<u64>,
}

impl AgentActivityClock {
    pub(super) fn seeded(seeds: Vec<AgentStateSince>) -> Self {
        Self {
            seeds,
            ..Self::default()
        }
    }

    pub(super) fn observe(&mut self, boot_id: &str, agents: &[ClientShellAgent], now: SystemTime) {
        let first = self.boot_id.as_deref() != Some(boot_id);
        if first {
            self.boot_id = Some(boot_id.to_owned());
            self.dirty |= !self.entries.is_empty();
            self.entries.clear();
        }
        let seeds = std::mem::take(&mut self.seeds);
        for agent in agents {
            let seq = agent.state_change_seq;
            match self.entries.get_mut(&agent.pane_id) {
                Some(entry) if entry.0 == seq => {}
                Some(entry) => {
                    *entry = (seq, Some(now));
                    self.dirty = true;
                }
                None => {
                    // An agent already running when this client attached has
                    // no known start unless a saved stamp matches it exactly.
                    let since = if first {
                        seeds
                            .iter()
                            .find(|seed| {
                                seed.boot_id == boot_id
                                    && seed.pane_id == agent.pane_id
                                    && seed.seq == seq
                            })
                            .map(|seed| UNIX_EPOCH + Duration::from_millis(seed.unix_ms))
                    } else {
                        Some(now)
                    };
                    self.entries.insert(agent.pane_id.clone(), (seq, since));
                    self.dirty |= since.is_some();
                }
            }
        }
        let before = self.entries.len();
        self.entries
            .retain(|pane_id, _| agents.iter().any(|agent| agent.pane_id == *pane_id));
        self.dirty |= self.entries.len() != before;
    }

    pub(super) fn since(&self, pane_id: &str) -> Option<SystemTime> {
        self.entries.get(pane_id).and_then(|(_, since)| *since)
    }

    pub(super) fn persisted(&self) -> Vec<AgentStateSince> {
        let Some(boot_id) = self.boot_id.as_deref() else {
            return Vec::new();
        };
        let mut stamps = self
            .entries
            .iter()
            .filter_map(|(pane_id, (seq, since))| {
                let unix_ms = (*since)?.duration_since(UNIX_EPOCH).ok()?.as_millis();
                Some(AgentStateSince {
                    boot_id: boot_id.to_owned(),
                    pane_id: pane_id.clone(),
                    seq: *seq,
                    unix_ms: u64::try_from(unix_ms).ok()?,
                })
            })
            .collect::<Vec<_>>();
        stamps.sort_by(|left, right| left.pane_id.cmp(&right.pane_id));
        stamps
    }

    /// Whether unsaved stamps are due for writing; marks them written.
    pub(super) fn take_due_persist(&mut self, now: Instant) -> bool {
        if !self.dirty
            || self
                .last_persist
                .is_some_and(|last| now.duration_since(last) < PERSIST_INTERVAL)
        {
            return false;
        }
        self.dirty = false;
        self.last_persist = Some(now);
        true
    }

    /// True once per wall-clock minute, so displayed ages can be redrawn.
    pub(super) fn minute_changed(&mut self, now: SystemTime) -> bool {
        let minute = now
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() / 60);
        self.minute.replace(minute) != Some(minute)
    }

    #[cfg(test)]
    pub(super) fn stamp_for_test(&mut self, pane_id: &str, seq: u64, since: SystemTime) {
        self.entries.insert(pane_id.to_owned(), (seq, Some(since)));
    }
}

/// Compact age such as `now`, `4m`, `2h` or `3d`, with configurable units.
pub(super) fn format_age(
    cards: &crate::config::CardsSidebarConfig,
    since: SystemTime,
    now: SystemTime,
) -> String {
    let seconds = now.duration_since(since).map_or(0, |age| age.as_secs());
    if seconds < 60 {
        return cards.label("now", "now").to_owned();
    }
    let (value, key, fallback) = if seconds < 3600 {
        (seconds / 60, "minutes", "m")
    } else if seconds < 86_400 {
        (seconds / 3600, "hours", "h")
    } else {
        (seconds / 86_400, "days", "d")
    };
    format!("{value}{}", cards.label(key, fallback))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::AgentStatus;

    fn agent(pane_id: &str, seq: u64) -> ClientShellAgent {
        ClientShellAgent {
            pane_id: pane_id.into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: None,
            display_agent: None,
            agent: None,
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: AgentStatus::Idle,
            state_change_seq: seq,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: false,
        }
    }

    #[test]
    fn stamps_changes_and_leaves_agents_seen_at_attach_unknown() {
        let start = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut clock = AgentActivityClock::seeded(vec![AgentStateSince {
            boot_id: "boot".into(),
            pane_id: "pane_2".into(),
            seq: 7,
            unix_ms: 999_000_000,
        }]);
        clock.observe("boot", &[agent("pane_1", 3), agent("pane_2", 7)], start);
        assert_eq!(clock.since("pane_1"), None);
        assert_eq!(
            clock.since("pane_2"),
            Some(UNIX_EPOCH + Duration::from_secs(999_000))
        );

        let later = start + Duration::from_secs(90);
        clock.observe(
            "boot",
            &[agent("pane_1", 4), agent("pane_2", 7), agent("pane_3", 1)],
            later,
        );
        assert_eq!(clock.since("pane_1"), Some(later));
        assert_eq!(clock.since("pane_3"), Some(later));
        assert_eq!(clock.persisted().len(), 3);

        clock.observe("boot", &[agent("pane_1", 4)], later);
        assert_eq!(clock.since("pane_2"), None);
        // A restarted server reuses pane ids, so old stamps are dropped.
        clock.observe("other", &[agent("pane_1", 4)], later);
        assert_eq!(clock.since("pane_1"), None);
    }

    #[test]
    fn persists_at_most_every_interval() {
        let mut clock = AgentActivityClock::default();
        let now = Instant::now();
        assert!(!clock.take_due_persist(now));
        clock.observe("boot", &[agent("pane_1", 1)], SystemTime::now());
        clock.observe("boot", &[agent("pane_1", 2)], SystemTime::now());
        assert!(clock.take_due_persist(now));
        clock.observe("boot", &[agent("pane_1", 3)], SystemTime::now());
        assert!(!clock.take_due_persist(now + Duration::from_secs(1)));
        assert!(clock.take_due_persist(now + PERSIST_INTERVAL));
    }

    #[test]
    fn ages_use_compact_units() {
        let mut cards = crate::config::CardsSidebarConfig::default();
        let since = UNIX_EPOCH + Duration::from_secs(10_000);
        let at = |seconds| since + Duration::from_secs(seconds);
        assert_eq!(format_age(&cards, since, at(30)), "now");
        assert_eq!(format_age(&cards, since, at(150)), "2m");
        assert_eq!(format_age(&cards, since, at(7_300)), "2h");
        assert_eq!(format_age(&cards, since, at(200_000)), "2d");
        cards.labels.insert("minutes".into(), "м".into());
        assert_eq!(format_age(&cards, since, at(150)), "2м");
    }
}
