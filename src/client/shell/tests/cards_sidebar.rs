use super::*;
use std::time::{Duration, SystemTime};

fn agent(pane_id: &str, workspace_id: &str, tab_id: &str, status: AgentStatus) -> ClientShellAgent {
    ClientShellAgent {
        pane_id: pane_id.into(),
        workspace_id: workspace_id.into(),
        tab_id: tab_id.into(),
        name: None,
        display_agent: Some("claude".into()),
        agent: None,
        title: None,
        terminal_title: None,
        terminal_title_stripped: Some("fix sidebar".into()),
        agent_status: status,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: pane_id == "pane_1",
    }
}

/// herdr-orca (waiting) with two tabs, bitrix (working), notes (no agent).
fn fleet() -> ClientShellSnapshot {
    let mut projected = snapshot();
    let base = projected.workspaces[0].clone();
    projected.workspaces = [
        ("ws_1", "herdr-orca", AgentStatus::Blocked),
        ("ws_2", "bitrix", AgentStatus::Working),
        ("ws_3", "notes", AgentStatus::Idle),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (id, label, status))| ClientShellWorkspace {
        workspace_id: id.into(),
        active_tab_id: format!("tab_{}", [1, 3, 4][index]),
        number: index + 1,
        label: label.into(),
        branch: None,
        focused: index == 0,
        agent_status: status,
        ..base.clone()
    })
    .collect();
    let tab = projected.tabs[0].clone();
    projected.tabs = [
        ("tab_1", "ws_1", "claude"),
        ("tab_2", "ws_1", "build"),
        ("tab_3", "ws_2", "1"),
        ("tab_4", "ws_3", "1"),
    ]
    .into_iter()
    .map(|(tab_id, workspace_id, label)| ClientShellTab {
        tab_id: tab_id.into(),
        workspace_id: workspace_id.into(),
        label: label.into(),
        focused: tab_id == "tab_1",
        ..tab.clone()
    })
    .collect();
    let pane = projected.panes[0].clone();
    projected.panes = [
        ("pane_1", "ws_1", "tab_1"),
        ("pane_2", "ws_1", "tab_2"),
        ("pane_3", "ws_1", "tab_2"),
        ("pane_4", "ws_2", "tab_3"),
        ("pane_5", "ws_3", "tab_4"),
    ]
    .into_iter()
    .map(|(pane_id, workspace_id, tab_id)| ClientShellPane {
        pane_id: pane_id.into(),
        workspace_id: workspace_id.into(),
        tab_id: tab_id.into(),
        focused: pane_id == "pane_1",
        foreground_cwd: Some(format!("/src/{pane_id}")),
        ..pane.clone()
    })
    .collect();
    projected.agents = vec![
        agent("pane_1", "ws_1", "tab_1", AgentStatus::Blocked),
        agent("pane_4", "ws_2", "tab_3", AgentStatus::Working),
    ];
    projected
}

fn cards_state() -> ClientShellState {
    let mut config = ClientShellConfig::from_config(&Config::default());
    config.sidebar_layout = crate::config::SidebarLayoutConfig::Cards;
    config.sidebar_width = 40;
    config.status_indicators = crate::config::StatusIndicatorStyle::Symbols;
    let mut state = ClientShellState::new(config);
    state.sidebar_width = 40;
    state.set_snapshot(Box::new(fleet()));
    state.set_pane_surface(surface());
    state
}

fn sidebar_rows(state: &mut ClientShellState) -> Vec<String> {
    sidebar_rows_at(state, 40)
}

fn sidebar_rows_at(state: &mut ClientShellState, height: u16) -> Vec<String> {
    let frame = state.compose(120, height).expect("frame");
    let width = usize::from(state.hits.sidebar_divider.x);
    frame_rows(&frame)
        .into_iter()
        .map(|row| {
            row.chars()
                .take(width)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

fn row_of(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not in sidebar: {rows:#?}"))
}

fn workspace_hit(state: &ClientShellState, workspace_id: &str) -> Rect {
    state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.workspace_id == workspace_id)
        .map(|hit| hit.rect)
        .unwrap_or_else(|| panic!("no card for {workspace_id}"))
}

fn endpoint_methods(outcome: &ClientShellInput) -> Vec<&crate::api::schema::Method> {
    outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => Some(&request.method),
            _ => None,
        })
        .collect()
}

#[test]
fn summary_counts_jump_to_the_next_agent_in_that_state() {
    let mut state = cards_state();
    sidebar_rows(&mut state);
    let (rect, _) = state
        .hits
        .card_status
        .iter()
        .find(|(_, status)| *status == AgentStatus::Working)
        .cloned()
        .expect("working chip");
    let mut outcome = ClientShellInput::default();
    assert!(state.handle_card_sidebar_click((rect.x, rect.y), &mut outcome));
    assert!(matches!(
        endpoint_methods(&outcome).as_slice(),
        [crate::api::schema::Method::PaneFocus(target)] if target.pane_id == "pane_4"
    ));
}

#[test]
fn pinned_cards_come_first_and_the_filter_narrows_the_list() {
    let mut state = cards_state();
    state.toggle_pinned_workspace("ws_3".into());
    let rows = sidebar_rows(&mut state);
    let notes = row_of(&rows, "notes");
    let orca = row_of(&rows, "herdr-orca");
    assert_eq!(notes, 4, "{rows:#?}");
    // One blank row separates pinned cards from the rest; notes has a name,
    // a path and one terminal row.
    assert!(rows[notes + 3].is_empty(), "{rows:#?}");
    assert!(rows[notes + 4].contains('╭'), "{rows:#?}");
    assert_eq!(orca, notes + 5, "{rows:#?}");

    state.sidebar_filter = TextEditor::from("BIT");
    let rows = sidebar_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("bitrix")));
    assert!(!rows.iter().any(|row| row.contains("herdr-orca")));
    assert!(!rows.iter().any(|row| row.contains("notes")));
    // The filter stays in the search field while it is set.
    assert!(rows[0].contains("BIT"), "{rows:#?}");
    let order = state.card_navigation_entries(state.snapshot.as_deref().unwrap());
    assert_eq!(order.len(), 1);
    assert_eq!(order[0].index, 1);

    // Agent titles are searchable too.
    state.sidebar_filter = TextEditor::from("fix sidebar");
    let order = state.card_navigation_entries(state.snapshot.as_deref().unwrap());
    assert_eq!(
        order.iter().map(|entry| entry.index).collect::<Vec<_>>(),
        vec![0, 1]
    );

    state.toggle_pinned_workspace("ws_3".into());
    assert!(state.pinned_workspaces.is_empty());
}

#[test]
fn the_group_button_toggles_repository_sections_that_collapse() {
    let mut state = cards_state();
    sidebar_rows(&mut state);
    let toggle = state.hits.card_group_toggle;
    let mut outcome = ClientShellInput::default();
    assert!(state.handle_card_sidebar_click((toggle.x, toggle.y), &mut outcome));
    assert_eq!(
        state.config.cards.group_by,
        crate::config::CardsGroupBy::Project
    );
    assert!(state.cards_group_by_manual);
    let rows = sidebar_rows(&mut state);
    let section = row_of(&rows, "▾ no repo");
    assert!(rows[section].ends_with('3'), "{rows:#?}");
    assert!(section < row_of(&rows, "herdr-orca"));

    // A collapsed section keeps only the focused card.
    let (rect, key) = state.hits.card_sections[0].clone();
    assert!(state.handle_card_sidebar_click((rect.x + 2, rect.y), &mut outcome));
    assert!(state.collapsed_groups.contains(&key));
    let rows = sidebar_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("▸ no repo")));
    assert!(rows.iter().any(|row| row.contains("herdr-orca")));
    assert!(!rows.iter().any(|row| row.contains("bitrix")), "{rows:#?}");

    assert!(state.handle_card_sidebar_click((toggle.x, toggle.y), &mut outcome));
    assert_eq!(
        state.config.cards.group_by,
        crate::config::CardsGroupBy::None
    );
    let rows = sidebar_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("bitrix")), "{rows:#?}");
}

#[test]
fn the_search_button_opens_a_filter_that_takes_typing() {
    let mut state = cards_state();
    sidebar_rows(&mut state);
    let search = state.hits.card_filter_toggle;
    let mut outcome = ClientShellInput::default();
    assert!(state.handle_card_sidebar_click((search.x, search.y), &mut outcome));
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::SidebarFilter)
    ));

    let outcome = state.handle_input_bytes(b"not");
    assert!(
        outcome.requests.is_empty(),
        "typing must not reach the pane"
    );
    assert_eq!(state.sidebar_filter.as_str(), "not");
    let frame = state.compose(120, 40).expect("frame");
    let filter = state.hits.card_filter;
    assert_eq!(filter.y, 0);
    assert!(frame
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.y == filter.y));

    state.handle_input_bytes(b"\r");
    assert!(state.overlay.is_none());
    assert_eq!(state.sidebar_filter.as_str(), "not");
    let rows = sidebar_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("notes")));
    assert!(!rows.iter().any(|row| row.contains("bitrix")));

    state.begin_sidebar_filter();
    state.handle_input_bytes(b"\x1b");
    assert!(state.overlay.is_none());
    assert!(state.sidebar_filter.is_empty());
    let rows = sidebar_rows(&mut state);
    assert!(rows[0].contains("search"), "{rows:#?}");
    assert!(rows[2].contains("× 1"), "{rows:#?}");
}

#[test]
fn cards_drag_in_workspace_order_only() {
    let mut state = cards_state();
    assert!(state.cards_layout_active());
    sidebar_rows(&mut state);
    let press = ClientWorkspacePress {
        endpoint_id: ClientEndpointId::Local,
        workspace_id: "ws_2".into(),
        start_column: 0,
        start_row: 0,
    };
    assert!(state.endpoint_workspace_is_draggable(&press));

    // Dropping on the row above bitrix moves a card in front of it.
    let bitrix = workspace_hit(&state, "ws_2");
    assert_eq!(
        state.card_drop_target_at((5, bitrix.y - 1)),
        Some((Some("ws_2".into()), bitrix.y - 1))
    );
    let notes = workspace_hit(&state, "ws_3");
    assert_eq!(
        state.card_drop_target_at((5, notes.bottom())),
        Some((None, notes.bottom()))
    );

    state.toggle_pinned_workspace("ws_2".into());
    assert!(!state.endpoint_workspace_is_draggable(&press));
    state.toggle_pinned_workspace("ws_2".into());
    state.config.cards.group_by = crate::config::CardsGroupBy::Project;
    assert!(!state.endpoint_workspace_is_draggable(&press));
}

#[test]
fn cards_layout_keeps_classic_sidebar_when_collapsed() {
    let mut state = cards_state();
    state.sidebar_collapsed = true;
    assert!(!state.cards_layout_active());
    let rows = sidebar_rows(&mut state);
    assert!(!rows.iter().any(|row| row.contains("WORKSPACES")));
}

#[test]
fn pin_entry_appears_in_the_workspace_menu_only_for_cards() {
    let mut state = cards_state();
    sidebar_rows(&mut state);
    state.open_workspace_context_menu("ws_2".into(), 1, 1);
    let Some(ClientShellOverlay::ContextMenu(menu)) = state.overlay.as_ref() else {
        panic!("context menu");
    };
    assert_eq!(menu.items()[0].label, "Pin");
    state.activate_context_menu_item(0, &mut ClientShellInput::default());
    assert_eq!(state.pinned_workspaces, vec!["ws_2".to_owned()]);
    state.open_workspace_context_menu("ws_2".into(), 1, 1);
    let Some(ClientShellOverlay::ContextMenu(menu)) = state.overlay.as_ref() else {
        panic!("context menu");
    };
    assert_eq!(menu.items()[0].label, "Unpin");

    state.overlay = None;
    state.config.sidebar_layout = crate::config::SidebarLayoutConfig::Panels;
    state.open_workspace_context_menu("ws_2".into(), 1, 1);
    let Some(ClientShellOverlay::ContextMenu(menu)) = state.overlay.as_ref() else {
        panic!("context menu");
    };
    assert!(menu.items().iter().all(|item| item.label != "Pin"));
}

fn session_ids<'a>(index: &'a cards_rows::SessionIndex<'a>, workspace_id: &str) -> Vec<&'a str> {
    index
        .sessions(workspace_id)
        .rows
        .iter()
        .map(|row| row.pane_id)
        .collect()
}

#[test]
fn sessions_list_agents_then_terminals_in_tab_order() {
    let snapshot = fleet();
    let index = cards_rows::SessionIndex::new(&snapshot, "");
    let orca = index.sessions("ws_1");
    assert_eq!(orca.agents, 1);
    assert_eq!(orca.terminals, 2);
    assert_eq!(session_ids(&index, "ws_1"), ["pane_1", "pane_2", "pane_3"]);
    assert!(orca.rows[0].focused);
    assert!(!orca.rows[1].focused);
    let notes = index.sessions("ws_3");
    assert_eq!(notes.agents, 0);
    assert_eq!(notes.terminals, 1);
    assert!(index.sessions("missing").rows.is_empty());
}

#[test]
fn a_status_change_does_not_reorder_rows() {
    let mut snapshot = fleet();
    snapshot
        .agents
        .push(agent("pane_3", "ws_1", "tab_2", AgentStatus::Done));
    {
        let index = cards_rows::SessionIndex::new(&snapshot, "");
        assert_eq!(session_ids(&index, "ws_1"), ["pane_1", "pane_3", "pane_2"]);
    }
    snapshot.agents[0].agent_status = AgentStatus::Idle;
    snapshot.agents[2].agent_status = AgentStatus::Blocked;
    let index = cards_rows::SessionIndex::new(&snapshot, "");
    assert_eq!(session_ids(&index, "ws_1"), ["pane_1", "pane_3", "pane_2"]);
}

#[test]
fn the_filter_puts_matching_sessions_first() {
    let snapshot = fleet();
    let index = cards_rows::SessionIndex::new(&snapshot, "pane_3");
    assert_eq!(session_ids(&index, "ws_1"), ["pane_3", "pane_1", "pane_2"]);
}

#[test]
fn row_window_caps_and_keeps_the_focused_row() {
    use cards_rows::{row_window, RowWindow};
    let window = |start, len, hidden| RowWindow { start, len, hidden };
    assert_eq!(row_window(3, 3, None), window(0, 3, 0));
    // A "1 more" line would take the same row as the row itself.
    assert_eq!(row_window(4, 3, None), window(0, 4, 0));
    assert_eq!(row_window(5, 3, None), window(0, 3, 2));
    assert_eq!(row_window(10, 3, Some(7)), window(5, 3, 7));
    assert_eq!(row_window(10, 1, Some(9)), window(9, 1, 9));
    assert_eq!(row_window(2, 0, Some(1)), window(0, 0, 0));
}

#[test]
fn counts_use_plural_labels() {
    let mut state = cards_state();
    let words = ["agent", "agents", "agents"];
    assert_eq!(
        cards_rows::count_label(&state.config.cards, "agents", 1, words),
        "1 agent"
    );
    assert_eq!(
        cards_rows::count_label(&state.config.cards, "agents", 3, words),
        "3 agents"
    );
    for (key, word) in [
        ("agents_one", "агент"),
        ("agents_few", "агента"),
        ("agents_many", "агентов"),
    ] {
        state.config.cards.labels.insert(key.into(), word.into());
    }
    let counted = [1, 2, 5, 11, 21, 22]
        .map(|count| cards_rows::count_label(&state.config.cards, "agents", count, words));
    assert_eq!(
        counted,
        [
            "1 агент",
            "2 агента",
            "5 агентов",
            "11 агентов",
            "21 агент",
            "22 агента"
        ]
    );
}

#[test]
fn claude_has_its_own_glyph() {
    let state = cards_state();
    let mut claude = agent("pane_1", "ws_1", "tab_1", AgentStatus::Idle);
    assert_eq!(
        cards_rows::agent_glyph(&claude, &state.config.palette).0,
        "✳"
    );
    claude.display_agent = Some("mystery".into());
    assert_eq!(
        cards_rows::agent_glyph(&claude, &state.config.palette).0,
        "◆"
    );
}

#[test]
fn the_header_has_a_search_field_and_counters_beside_the_title() {
    let mut state = cards_state();
    let rows = sidebar_rows(&mut state);
    assert!(
        rows[0].contains('⌕') && rows[0].contains("search"),
        "{rows:#?}"
    );
    assert!(rows[0].ends_with('≡'), "{rows:#?}");
    assert!(rows[1].is_empty(), "{rows:#?}");
    assert!(rows[2].contains("WORKSPACES"), "{rows:#?}");
    assert!(
        rows[2].contains("× 1") && rows[2].contains("◐ 1"),
        "{rows:#?}"
    );
    assert!(rows[3].is_empty(), "{rows:#?}");
    // The search field and the counters are click targets on their rows.
    assert_eq!(state.hits.card_filter_toggle.y, 0);
    assert!(state.hits.card_status.iter().all(|(rect, _)| rect.y == 2));
}

/// fleet() with `count` more agents in bitrix's tab, titled "task N".
fn fleet_with_bitrix_agents(count: usize) -> ClientShellSnapshot {
    let mut snapshot = fleet();
    let pane = snapshot.panes[3].clone();
    for n in 0..count {
        let pane_id = format!("pane_b{n}");
        snapshot.panes.push(ClientShellPane {
            pane_id: pane_id.clone(),
            focused: false,
            ..pane.clone()
        });
        let mut extra = agent(&pane_id, "ws_2", "tab_3", AgentStatus::Done);
        extra.terminal_title_stripped = Some(format!("task {n}"));
        snapshot.agents.push(extra);
    }
    snapshot
}

fn session_hit(state: &ClientShellState, pane_id: &str) -> Rect {
    state
        .hits
        .card_sessions
        .iter()
        .find(|(_, id)| id == pane_id)
        .map(|(rect, _)| *rect)
        .unwrap_or_else(|| panic!("no session row for {pane_id}"))
}

#[test]
fn the_focused_card_is_framed_and_lists_agents_then_terminals() {
    let mut state = cards_state();
    let rows = sidebar_rows(&mut state);
    let orca = row_of(&rows, "herdr-orca");
    assert!(
        rows[orca - 1].contains('╭') && rows[orca - 1].ends_with('╮'),
        "{rows:#?}"
    );
    assert!(rows[orca].contains("│ × herdr-orca"), "{rows:#?}");
    assert!(rows[orca + 1].contains("/src/pane_1"), "{rows:#?}");
    assert!(
        rows[orca + 2].contains("▾ 1 agent · 2 terminals"),
        "{rows:#?}"
    );
    assert!(rows[orca + 3].contains("▎  × ✳ fix sidebar"), "{rows:#?}");
    assert!(rows[orca + 4].contains("$ build · pane_2"), "{rows:#?}");
    assert!(rows[orca + 5].contains("$ build · pane_3"), "{rows:#?}");
    assert!(
        rows[orca + 6].contains('╰') && rows[orca + 6].ends_with('╯'),
        "{rows:#?}"
    );

    // Workspace order, not state order, with one blank row between cards.
    let bitrix = row_of(&rows, "bitrix");
    let notes = row_of(&rows, "notes");
    assert_eq!(bitrix, orca + 8, "{rows:#?}");
    assert!(bitrix < notes, "{rows:#?}");
    // Other cards have no frame and list their sessions too.
    assert!(
        rows[bitrix].contains("◐ bitrix") && !rows[bitrix].contains('│'),
        "{rows:#?}"
    );
    assert!(rows[bitrix + 1].contains("/src/pane_4"), "{rows:#?}");
    assert!(rows[bitrix + 2].contains("◐ ✳ fix sidebar"), "{rows:#?}");
    assert!(rows[notes + 2].contains("$ 1 · pane_5"), "{rows:#?}");
    assert_eq!(
        state
            .card_navigation_entries(state.snapshot.as_deref().unwrap())
            .iter()
            .map(|entry| entry.index)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[test]
fn inactive_cards_show_capped_sessions() {
    let mut state = cards_state();
    state.set_snapshot(Box::new(fleet_with_bitrix_agents(4)));
    let rows = sidebar_rows(&mut state);
    let bitrix = row_of(&rows, "bitrix");
    assert!(rows[bitrix + 4].contains("task 1"), "{rows:#?}");
    assert!(rows[bitrix + 5].contains("… more 2"), "{rows:#?}");
    assert!(!rows.iter().any(|row| row.contains("task 2")), "{rows:#?}");

    // One hidden row is shown rather than summed up.
    state.set_snapshot(Box::new(fleet_with_bitrix_agents(3)));
    let rows = sidebar_rows(&mut state);
    let bitrix = row_of(&rows, "bitrix");
    assert!(rows[bitrix + 5].contains("task 2"), "{rows:#?}");
    assert!(!rows.iter().any(|row| row.contains("more")), "{rows:#?}");
}

#[test]
fn session_rows_focus_their_pane_and_cards_select_their_workspace() {
    let mut state = cards_state();
    let rows = sidebar_rows(&mut state);
    let bitrix = row_of(&rows, "bitrix");
    let row = session_hit(&state, "pane_4");
    assert_eq!(usize::from(row.y), bitrix + 2);
    let mut outcome = ClientShellInput::default();
    assert!(state.handle_card_sidebar_click((row.x + 12, row.y), &mut outcome));
    assert!(matches!(
        endpoint_methods(&outcome).as_slice(),
        [crate::api::schema::Method::PaneFocus(target)] if target.pane_id == "pane_4"
    ));
    // The card's own lines fall through to the workspace hit.
    let card = workspace_hit(&state, "ws_2");
    assert_eq!(card.height, 3);
    assert_eq!(card.width, state.hits.sidebar_divider.x);
    let mut outcome = ClientShellInput::default();
    assert!(!state.handle_card_sidebar_click((card.x + 6, card.y), &mut outcome));
}

#[test]
fn right_click_on_a_session_opens_its_pane_menu() {
    let mut state = cards_state();
    sidebar_rows(&mut state);
    let row = session_hit(&state, "pane_2");
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: row.x + 10,
        row: row.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
            target: ClientContextMenuTarget::Pane { ref pane_id, .. },
            ..
        })) if pane_id == "pane_2"
    ));
}

#[test]
fn a_tall_focused_card_fits_and_keeps_the_focused_row() {
    let mut state = cards_state();
    let mut snapshot = fleet();
    let pane = snapshot.panes[0].clone();
    for n in 0..9 {
        let pane_id = format!("pane_a{n}");
        snapshot.panes.push(ClientShellPane {
            pane_id: pane_id.clone(),
            focused: false,
            ..pane.clone()
        });
        let mut extra = agent(&pane_id, "ws_1", "tab_1", AgentStatus::Working);
        extra.terminal_title_stripped = Some(format!("job {n}"));
        snapshot.agents.push(extra);
    }
    snapshot.focused_pane_id = Some("pane_a8".into());
    state.set_snapshot(Box::new(snapshot));
    let rows = sidebar_rows_at(&mut state, 16);
    assert!(
        rows.iter().any(|row| row.contains("▎  ◐ ✳ job 8")),
        "{rows:#?}"
    );
    assert!(rows.iter().any(|row| row.contains("… more 7")), "{rows:#?}");
    assert!(rows.iter().any(|row| row.contains('╰')), "{rows:#?}");
}

#[test]
fn ages_sit_on_agent_rows() {
    let mut state = cards_state();
    let rows = sidebar_rows(&mut state);
    let bitrix = row_of(&rows, "bitrix");
    // Agents already running when the client attached have no known start.
    assert!(rows[bitrix + 2].ends_with("fix sidebar"), "{rows:#?}");

    state.cards_clock.stamp_for_test(
        "pane_4",
        1,
        SystemTime::now() - Duration::from_secs(5 * 60 + 10),
    );
    let rows = sidebar_rows(&mut state);
    assert!(rows[bitrix + 2].ends_with("5m"), "{rows:#?}");
    state
        .config
        .cards
        .labels
        .insert("minutes".into(), "м".into());
    let rows = sidebar_rows(&mut state);
    assert!(rows[bitrix + 2].ends_with("5м"), "{rows:#?}");
}

#[test]
fn narrow_sidebar_keeps_the_frame_inside() {
    let mut state = cards_state();
    state.config.sidebar_width = 24;
    state.sidebar_width = 24;
    let rows = sidebar_rows(&mut state);
    let top = rows
        .iter()
        .find(|row| row.contains('╭'))
        .unwrap_or_else(|| panic!("no frame: {rows:#?}"));
    assert!(top.ends_with('╮'), "{rows:#?}");
    assert!(
        rows.iter()
            .any(|row| row.contains('╰') && row.ends_with('╯')),
        "{rows:#?}"
    );
}

#[test]
fn long_titles_truncate_before_the_age() {
    let mut state = cards_state();
    let mut snapshot = fleet();
    snapshot.agents[1].terminal_title_stripped = Some("漢".repeat(40));
    state.set_snapshot(Box::new(snapshot));
    state.cards_clock.stamp_for_test(
        "pane_4",
        1,
        SystemTime::now() - Duration::from_secs(5 * 60 + 10),
    );
    let rows = sidebar_rows(&mut state);
    let bitrix = row_of(&rows, "bitrix");
    assert!(rows[bitrix + 2].contains('…'), "{rows:#?}");
    assert!(rows[bitrix + 2].ends_with("5m"), "{rows:#?}");
}

#[test]
fn the_counter_row_collapses_sessions_and_it_sticks() {
    let mut state = cards_state();
    state.collapsed_groups.insert("cards:sessions:gone".into());
    sidebar_rows(&mut state);
    let (toggle, workspace_id) = state.hits.card_session_toggles[0].clone();
    assert_eq!(workspace_id, "ws_1");
    let mut outcome = ClientShellInput::default();
    assert!(state.handle_card_sidebar_click((toggle.x + 6, toggle.y), &mut outcome));
    assert!(outcome.repaint);
    assert!(state.collapsed_groups.contains("cards:sessions:ws_1"));
    // Keys of closed workspaces are dropped.
    assert!(!state.collapsed_groups.contains("cards:sessions:gone"));
    let rows = sidebar_rows(&mut state);
    let orca = row_of(&rows, "herdr-orca");
    assert!(
        rows[orca + 2].contains("▸ 1 agent · 2 terminals"),
        "{rows:#?}"
    );
    assert!(rows[orca + 3].contains('╰'), "{rows:#?}");
    assert!(state
        .hits
        .card_sessions
        .iter()
        .all(|(_, pane)| pane != "pane_1"));

    // The card stays folded while another card is focused: name and path only.
    let mut snapshot = fleet();
    snapshot.workspaces[0].focused = false;
    snapshot.workspaces[1].focused = true;
    snapshot.focused_workspace_id = Some("ws_2".into());
    snapshot.focused_pane_id = Some("pane_4".into());
    state.set_snapshot(Box::new(snapshot));
    let rows = sidebar_rows(&mut state);
    assert_eq!(workspace_hit(&state, "ws_1").height, 2, "{rows:#?}");

    // A filter shows the sessions anyway, including terminals found by the
    // folder name their row shows.
    state.sidebar_filter = TextEditor::from("pane");
    let rows = sidebar_rows(&mut state);
    let orca = row_of(&rows, "herdr-orca");
    assert!(rows[orca + 3].contains("$ build · pane_2"), "{rows:#?}");

    // Clicking the counter again unfolds it.
    state.sidebar_filter = TextEditor::from("");
    state.set_snapshot(Box::new(fleet()));
    sidebar_rows(&mut state);
    let (toggle, _) = state.hits.card_session_toggles[0].clone();
    assert!(state.handle_card_sidebar_click((toggle.x + 6, toggle.y), &mut outcome));
    assert!(!state.collapsed_groups.contains("cards:sessions:ws_1"));
    let rows = sidebar_rows(&mut state);
    assert!(
        rows.iter()
            .any(|row| row.contains("▾ 1 agent · 2 terminals")),
        "{rows:#?}"
    );
}

fn focus_notes(snapshot: &mut ClientShellSnapshot) {
    for workspace in &mut snapshot.workspaces {
        workspace.focused = workspace.workspace_id == "ws_3";
    }
    snapshot.focused_workspace_id = Some("ws_3".into());
    snapshot.focused_tab_id = Some("tab_4".into());
    snapshot.focused_pane_id = Some("pane_5".into());
}

#[test]
fn a_growing_focused_card_scrolls_into_view() {
    let mut state = cards_state();
    let mut snapshot = fleet();
    focus_notes(&mut snapshot);
    state.set_snapshot(Box::new(snapshot.clone()));
    // The list fits exactly: orca 5 rows, bitrix 3, notes 6, two gaps.
    let rows = sidebar_rows_at(&mut state, 21);
    assert!(rows.iter().any(|row| row.contains('╰')), "{rows:#?}");

    // Three splits in notes, the last one focused.
    let pane = snapshot.panes[4].clone();
    for n in 0..3 {
        snapshot.panes.push(ClientShellPane {
            pane_id: format!("pane_n{n}"),
            focused: false,
            foreground_cwd: Some(format!("/src/pane_n{n}")),
            ..pane.clone()
        });
    }
    snapshot.focused_pane_id = Some("pane_n2".into());
    state.set_snapshot(Box::new(snapshot));
    let rows = sidebar_rows_at(&mut state, 21);
    assert!(
        rows.iter().any(|row| row.contains("▎    $ 1 · pane_n2")),
        "{rows:#?}"
    );
    assert!(rows.iter().any(|row| row.contains('╰')), "{rows:#?}");
}

#[test]
fn unfolding_the_focused_card_scrolls_it_into_view() {
    let mut state = cards_state();
    let mut snapshot = fleet();
    focus_notes(&mut snapshot);
    let pane = snapshot.panes[4].clone();
    for n in 0..3 {
        snapshot.panes.push(ClientShellPane {
            pane_id: format!("pane_n{n}"),
            focused: false,
            foreground_cwd: Some(format!("/src/pane_n{n}")),
            ..pane.clone()
        });
    }
    state.set_snapshot(Box::new(snapshot));
    state.collapsed_groups.insert("cards:sessions:ws_3".into());
    sidebar_rows_at(&mut state, 21);
    let (toggle, _) = state.hits.card_session_toggles[0].clone();
    let mut outcome = ClientShellInput::default();
    assert!(state.handle_card_sidebar_click((toggle.x + 6, toggle.y), &mut outcome));
    let rows = sidebar_rows_at(&mut state, 21);
    assert!(rows.iter().any(|row| row.contains("pane_n2")), "{rows:#?}");
    assert!(rows.iter().any(|row| row.contains('╰')), "{rows:#?}");
}

#[test]
fn keyboard_navigation_reveals_the_whole_selected_card() {
    let mut state = cards_state();
    state.mode = ClientShellMode::Navigate;
    // orca takes 8 rows and the gap 1, so only bitrix's name fits.
    sidebar_rows_at(&mut state, 15);
    assert_eq!(workspace_hit(&state, "ws_2").height, 1);
    // The first step selects the focused card, the second moves to bitrix.
    state.move_navigate_workspace(1);
    sidebar_rows_at(&mut state, 15);
    state.move_navigate_workspace(1);
    let rows = sidebar_rows_at(&mut state, 15);
    assert_eq!(workspace_hit(&state, "ws_2").height, 3, "{rows:#?}");
}

#[test]
fn a_shorter_terminal_keeps_the_focused_card_in_view() {
    let mut state = cards_state();
    let mut snapshot = fleet();
    focus_notes(&mut snapshot);
    let pane = snapshot.panes[4].clone();
    for n in 0..3 {
        snapshot.panes.push(ClientShellPane {
            pane_id: format!("pane_n{n}"),
            focused: false,
            foreground_cwd: Some(format!("/src/pane_n{n}")),
            ..pane.clone()
        });
    }
    snapshot.focused_pane_id = Some("pane_n2".into());
    state.set_snapshot(Box::new(snapshot));
    sidebar_rows_at(&mut state, 40);
    let rows = sidebar_rows_at(&mut state, 21);
    assert!(rows.iter().any(|row| row.contains("pane_n2")), "{rows:#?}");
    assert!(rows.iter().any(|row| row.contains('╰')), "{rows:#?}");
}
