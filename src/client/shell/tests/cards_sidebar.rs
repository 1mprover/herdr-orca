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
    let frame = state.compose(120, 40).expect("frame");
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
fn cards_keep_workspace_order_and_the_focused_card_lists_its_tabs() {
    let mut state = cards_state();
    let rows = sidebar_rows(&mut state);

    assert!(rows[0].contains("WORKSPACES"), "{rows:#?}");
    assert!(rows[0].contains('⌕') && rows[0].contains('≡'), "{rows:#?}");
    assert!(rows[1].contains("× 1 waiting"), "{rows:#?}");
    assert!(rows[1].contains("◐ 1 working"), "{rows:#?}");
    assert!(rows[2].starts_with('─'), "{rows:#?}");

    // Workspace order, not state order: the waiting card is not pulled up
    // and the idle one is not pushed down.
    let orca = row_of(&rows, "1 herdr-orca");
    let bitrix = row_of(&rows, "2 bitrix");
    let notes = row_of(&rows, "3 notes");
    assert!(orca < bitrix && bitrix < notes, "{rows:#?}");
    assert!(rows[orca].starts_with('▌'), "{rows:#?}");
    assert!(rows[orca].ends_with('×'), "{rows:#?}");

    // The focused card unfolds into its tabs; a tab without an agent names
    // its panes.
    assert!(rows[orca + 1].contains("/src/pane_1"), "{rows:#?}");
    assert!(
        rows[orca + 2].contains("├ claude  × fix sidebar"),
        "{rows:#?}"
    );
    assert!(
        rows[orca + 3].contains("└ build   pane_2 · pane_3"),
        "{rows:#?}"
    );
    assert_eq!(bitrix, orca + 4, "{rows:#?}");

    // Other cards are two lines: what the lead agent is doing, else the path.
    assert!(rows[bitrix].ends_with('◐'), "{rows:#?}");
    assert!(
        rows[bitrix + 1].contains("claude · fix sidebar"),
        "{rows:#?}"
    );
    assert!(rows[notes + 1].contains("/src/pane_5"), "{rows:#?}");
    assert!(!rows.iter().any(|row| row.contains("WORKING")), "{rows:#?}");

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
fn the_whole_card_is_one_click_target_and_tab_lines_focus_their_tab() {
    let mut state = cards_state();
    let rows = sidebar_rows(&mut state);

    assert_eq!(state.hits.workspaces.len(), 3);
    let orca = workspace_hit(&state, "ws_1");
    let bitrix = workspace_hit(&state, "ws_2");
    assert_eq!(orca.height, 4);
    assert_eq!(bitrix.height, 2);
    assert_eq!(usize::from(bitrix.y), row_of(&rows, "2 bitrix"));
    // Every column of the card counts, not just the label.
    assert_eq!(bitrix.width, state.hits.sidebar_divider.x);

    let (tab_rect, tab_id) = state
        .hits
        .card_tabs
        .iter()
        .find(|(_, tab_id)| tab_id == "tab_2")
        .cloned()
        .expect("build tab line");
    assert_eq!(usize::from(tab_rect.y), row_of(&rows, "└ build"));
    let mut outcome = ClientShellInput::default();
    assert!(state.handle_card_sidebar_click((tab_rect.x + 20, tab_rect.y), &mut outcome));
    assert!(matches!(
        endpoint_methods(&outcome).as_slice(),
        [crate::api::schema::Method::TabFocus(target)] if target.tab_id == tab_id
    ));
    // The card's own lines fall through to the workspace hit.
    let mut outcome = ClientShellInput::default();
    assert!(!state.handle_card_sidebar_click((bitrix.x + 10, bitrix.y + 1), &mut outcome));
}

#[test]
fn ages_show_how_long_the_lead_agent_has_been_in_its_state() {
    let mut state = cards_state();
    let rows = sidebar_rows(&mut state);
    // Agents already running when the client attached have no known start.
    assert!(rows[row_of(&rows, "2 bitrix")].ends_with('◐'), "{rows:#?}");

    state.cards_clock.stamp_for_test(
        "pane_4",
        1,
        SystemTime::now() - Duration::from_secs(5 * 60 + 10),
    );
    let rows = sidebar_rows(&mut state);
    assert!(
        rows[row_of(&rows, "2 bitrix")].ends_with("◐ 5m"),
        "{rows:#?}"
    );

    state
        .config
        .cards
        .labels
        .insert("minutes".into(), "м".into());
    let rows = sidebar_rows(&mut state);
    assert!(
        rows[row_of(&rows, "2 bitrix")].ends_with("◐ 5м"),
        "{rows:#?}"
    );
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
    let notes = row_of(&rows, "3 notes");
    let orca = row_of(&rows, "1 herdr-orca");
    assert_eq!(notes, 3, "{rows:#?}");
    // One blank row separates pinned cards from the rest.
    assert_eq!(orca, notes + 3, "{rows:#?}");
    assert!(rows[notes + 2].is_empty(), "{rows:#?}");

    state.sidebar_filter = TextEditor::from("BIT");
    let rows = sidebar_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("bitrix")));
    assert!(!rows.iter().any(|row| row.contains("herdr-orca")));
    assert!(!rows.iter().any(|row| row.contains("notes")));
    // The filter takes the summary row's place while it is set.
    assert!(rows[1].contains("BIT"), "{rows:#?}");
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
    assert!(section < row_of(&rows, "1 herdr-orca"));

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
    assert_eq!(filter.y, 1);
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
    assert!(rows[1].contains("waiting"), "{rows:#?}");
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
