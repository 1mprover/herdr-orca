use super::*;

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

/// herdr-orca (needs input) with two tabs, bitrix (working), notes (idle).
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
        active_tab_id: format!("tab_{}", index + 1),
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
    frame_rows(&frame)
        .into_iter()
        .map(|row| {
            row.chars()
                .take(39)
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

#[test]
fn cards_group_workspaces_by_status_with_tab_and_pane_tree() {
    let mut state = cards_state();
    let rows = sidebar_rows(&mut state);

    assert!(rows[0].contains("workspaces") && rows[0].contains("status"));
    assert!(rows[1].contains("filter"));
    let blocked = row_of(&rows, "NEEDS INPUT · 1");
    let working = row_of(&rows, "WORKING · 1");
    let idle = row_of(&rows, "IDLE · 1");
    assert!(blocked < working && working < idle, "{rows:#?}");

    // Two tabs, so the card shows them, with panes under each.
    let card = row_of(&rows, "herdr-orca");
    assert!(card > blocked && card < working);
    let claude_tab = row_of(&rows, "├ claude");
    let agent_line = row_of(&rows, "└ × claude");
    let build_tab = row_of(&rows, "└ build");
    assert!(
        claude_tab < agent_line && agent_line < build_tab,
        "{rows:#?}"
    );
    assert!(rows[build_tab + 1].contains("├ › pane_2"), "{rows:#?}");
    assert!(rows[build_tab + 2].contains("└ › pane_3"), "{rows:#?}");
    // A single auto-named tab is skipped; its agent hangs off the card.
    let bitrix = row_of(&rows, "bitrix");
    assert!(rows[bitrix + 1].contains("└ ◐ claude"), "{rows:#?}");
    // A lone shell adds nothing.
    let notes = row_of(&rows, "notes");
    assert!(!rows[notes + 1].contains("›"), "{rows:#?}");

    assert_eq!(state.hits.workspaces.len(), 3);
    assert!(state
        .hits
        .card_tabs
        .iter()
        .any(|(_, tab_id)| tab_id == "tab_2"));
    assert!(state
        .hits
        .agents
        .iter()
        .any(|(_, pane_id)| pane_id == "pane_3"));
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
fn pinned_cards_come_first_and_the_filter_narrows_the_list() {
    let mut state = cards_state();
    state.toggle_pinned_workspace("ws_3".into());
    let rows = sidebar_rows(&mut state);
    let pinned = row_of(&rows, "PINNED · 1");
    assert!(pinned < row_of(&rows, "notes"));
    assert!(row_of(&rows, "notes") < row_of(&rows, "NEEDS INPUT"));
    assert!(!rows.iter().any(|row| row.contains("IDLE")), "{rows:#?}");

    state.sidebar_filter = TextEditor::from("BIT");
    let rows = sidebar_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("bitrix")));
    assert!(!rows.iter().any(|row| row.contains("herdr-orca")));
    assert!(!rows.iter().any(|row| row.contains("PINNED")));
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
fn section_headers_collapse_and_the_group_label_cycles() {
    let mut state = cards_state();
    sidebar_rows(&mut state);
    let (rect, key) = state
        .hits
        .card_sections
        .iter()
        .find(|(_, key)| key == "cards:status:working")
        .cloned()
        .expect("working section");
    let mut outcome = ClientShellInput::default();
    assert!(state.handle_card_sidebar_click((rect.x + 2, rect.y), &mut outcome));
    assert!(state.collapsed_groups.contains(&key));
    let rows = sidebar_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("▸ ◐ WORKING · 1")));
    assert!(!rows.iter().any(|row| row.contains("bitrix")), "{rows:#?}");

    // The focused card stays visible inside a collapsed section.
    let (rect, _) = state
        .hits
        .card_sections
        .iter()
        .find(|(_, key)| key == "cards:status:blocked")
        .cloned()
        .unwrap();
    state.handle_card_sidebar_click((rect.x + 2, rect.y), &mut outcome);
    let rows = sidebar_rows(&mut state);
    assert!(rows.iter().any(|row| row.contains("herdr-orca")));

    let toggle = state.hits.card_group_toggle;
    assert!(state.handle_card_sidebar_click((toggle.x, toggle.y), &mut outcome));
    assert_eq!(
        state.config.cards.group_by,
        crate::config::CardsGroupBy::Project
    );
    assert!(state.cards_group_by_manual);
    let rows = sidebar_rows(&mut state);
    assert!(rows[0].contains("project"));
    assert!(
        rows.iter().any(|row| row.contains("NO REPO · 3")),
        "{rows:#?}"
    );
}

#[test]
fn filter_field_takes_typing_until_enter_and_escape_clears_it() {
    let mut state = cards_state();
    sidebar_rows(&mut state);
    let filter = state.hits.card_filter;
    let mut outcome = ClientShellInput::default();
    assert!(state.handle_card_sidebar_click((filter.x + 4, filter.y), &mut outcome));
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
}

#[test]
fn cards_layout_keeps_classic_sidebar_when_collapsed_and_blocks_drag() {
    let mut state = cards_state();
    assert!(state.cards_layout_active());
    let press = ClientWorkspacePress {
        endpoint_id: ClientEndpointId::Local,
        workspace_id: "ws_2".into(),
        start_column: 0,
        start_row: 0,
    };
    assert!(!state.endpoint_workspace_is_draggable(&press));
    state.sidebar_collapsed = true;
    assert!(!state.cards_layout_active());
    let rows = sidebar_rows(&mut state);
    assert!(!rows.iter().any(|row| row.contains("workspaces")));
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
