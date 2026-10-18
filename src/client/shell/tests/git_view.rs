use super::*;
use crate::api::schema::{
    GitDiffResult, GitFileChange, GitSection, GitStatusResult, Method, ResponseResult,
};

fn state() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.set_endpoint_methods(Some(vec!["git.status".into(), "git.diff".into()]));
    state
}

fn request(outcome: &ClientShellInput) -> &crate::api::schema::Request {
    let ClientShellAction::Endpoint { request, .. } =
        outcome.actions.first().expect("endpoint request")
    else {
        panic!("endpoint action");
    };
    request
}

fn status() -> ResponseResult {
    ResponseResult::GitStatus {
        status: GitStatusResult {
            terminal_id: Some("term_original".into()),
            root: "/repo".into(),
            branch: "feature/git".into(),
            section: GitSection::Local,
            files: ["first.rs", "second.rs", "third.rs"]
                .map(|path| GitFileChange {
                    path: path.into(),
                    old_path: None,
                    status: "M".into(),
                    added: Some(2),
                    removed: Some(1),
                })
                .to_vec(),
        },
    }
}

fn diff() -> ResponseResult {
    ResponseResult::GitDiff {
        diff: GitDiffResult {
            text: "@@ -1 +1 @@\n-before\n+after\n".into(),
            truncated: false,
            binary: false,
        },
    }
}

#[test]
fn git_view_requires_both_explicit_capabilities() {
    for methods in [
        None,
        Some(vec!["git.status".into()]),
        Some(vec!["git.diff".into()]),
    ] {
        let mut state = state();
        state.set_endpoint_methods(methods);
        let mut outcome = ClientShellInput::default();
        state.open_git_view("pane_1".into(), &mut outcome);
        assert!(outcome.actions.is_empty());
        assert!(!matches!(
            state.overlay,
            Some(ClientShellOverlay::GitChanges(_))
        ));
    }
}

#[test]
fn git_view_coalesces_navigation_and_ignores_errors_after_close() {
    let mut state = state();
    let mut outcome = ClientShellInput::default();
    state.open_git_view("pane_1".into(), &mut outcome);
    assert!(matches!(request(&outcome).method, Method::GitStatus(_)));
    let (_, actions) = state.handle_endpoint_result("boot-1", &request(&outcome).id, Ok(status()));
    let initial_diff = ClientShellInput {
        actions,
        ..ClientShellInput::default()
    };
    assert!(
        matches!(&request(&initial_diff).method, Method::GitDiff(params) if params.path == "first.rs")
    );
    let down = crate::input::TerminalKey::new(KeyCode::Down, KeyModifiers::empty());
    let mut navigation = ClientShellInput::default();
    state.route_git_view_key(&down, &mut navigation);
    state.route_git_view_key(&down, &mut navigation);
    state.route_git_view_key(&down, &mut navigation);
    assert!(
        navigation.actions.is_empty(),
        "pending diff must coalesce rapid selections"
    );
    let (_, actions) =
        state.handle_endpoint_result("boot-1", &request(&initial_diff).id, Ok(diff()));
    let latest = ClientShellInput {
        actions,
        ..ClientShellInput::default()
    };
    assert!(
        matches!(&request(&latest).method, Method::GitDiff(params) if params.path == "third.rs")
    );
    state.overlay = None;
    let (repaint, actions) = state.handle_endpoint_result(
        "boot-1",
        &request(&latest).id,
        Err(ClientShellEndpointError {
            code: Some("endpoint_timeout".into()),
            message: "late timeout".into(),
        }),
    );
    assert!(!repaint);
    assert!(actions.is_empty());
    assert!(state.visible_endpoint_notice.is_none());
}

#[test]
fn git_view_renders_and_mouse_switches_sections_and_closes_when_small() {
    let mut state = state();
    let mut outcome = ClientShellInput::default();
    state.open_git_view("pane_1".into(), &mut outcome);
    let (_, actions) = state.handle_endpoint_result("boot-1", &request(&outcome).id, Ok(status()));
    let initial_diff = ClientShellInput {
        actions,
        ..ClientShellInput::default()
    };
    state.handle_endpoint_result("boot-1", &request(&initial_diff).id, Ok(diff()));
    let frame = state.compose(132, 36).unwrap();
    let text = frame_rows(&frame).join("\n");
    for expected in [
        "Git changes",
        "feature/git",
        "first.rs",
        "+after",
        "shared by all agents",
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
    assert_eq!(state.hits.git_files.len(), 3);
    let staged = state.hits.git_sections[1].0;
    let switch = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: staged.x,
        row: staged.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(
        matches!(&request(&switch).method, Method::GitStatus(params) if params.section == GitSection::Staged)
    );
    state.compose(42, 12).unwrap();
    let close = state.hits.overlay_cancel;
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: close.x,
        row: close.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(state.overlay.is_none());
}

#[test]
fn git_view_coalesces_refresh_and_does_not_show_old_section() {
    let mut state = state();
    let mut outcome = ClientShellInput::default();
    state.open_git_view("pane_1".into(), &mut outcome);
    let mut switch = ClientShellInput::default();
    state.route_git_view_key(
        &crate::input::TerminalKey::new(KeyCode::Tab, KeyModifiers::empty()),
        &mut switch,
    );
    assert!(switch.actions.is_empty());
    let (_, actions) = state.handle_endpoint_result("boot-1", &request(&outcome).id, Ok(status()));
    let latest = ClientShellInput {
        actions,
        ..ClientShellInput::default()
    };
    assert!(
        matches!(&request(&latest).method, Method::GitStatus(params) if params.section == GitSection::Staged && params.target == "term_original")
    );
    assert!(
        matches!(state.overlay, Some(ClientShellOverlay::GitChanges(ref view)) if view.status.is_none())
    );
}

#[test]
fn git_view_cancellation_does_not_start_a_deferred_refresh() {
    let mut state = state();
    let mut outcome = ClientShellInput::default();
    state.open_git_view("pane_1".into(), &mut outcome);
    state.route_git_view_key(
        &crate::input::TerminalKey::new(KeyCode::Char('r'), KeyModifiers::empty()),
        &mut ClientShellInput::default(),
    );
    state.cancel_endpoint_request(&request(&outcome).id);
    assert!(state.pending_requests.is_empty());
    assert!(
        matches!(state.overlay, Some(ClientShellOverlay::GitChanges(ref view)) if !view.loading && !view.refresh_pending)
    );
}

fn loaded_state(result: ResponseResult) -> ClientShellState {
    let mut state = state();
    let mut outcome = ClientShellInput::default();
    state.open_git_view("pane_1".into(), &mut outcome);
    let (_, actions) = state.handle_endpoint_result("boot-1", &request(&outcome).id, Ok(result));
    let initial_diff = ClientShellInput {
        actions,
        ..ClientShellInput::default()
    };
    state.handle_endpoint_result("boot-1", &request(&initial_diff).id, Ok(diff()));
    state
}

#[test]
fn git_view_keeps_terminal_target_when_pane_id_changes() {
    let mut state = loaded_state(status());
    let mut moved = snapshot();
    moved.revision += 1;
    moved.panes[0].pane_id = "pane_after_move".into();
    moved.focused_pane_id = Some("pane_after_move".into());
    state.set_snapshot(Box::new(moved));
    let mut outcome = ClientShellInput::default();
    state.route_git_view_key(
        &crate::input::TerminalKey::new(KeyCode::Down, KeyModifiers::empty()),
        &mut outcome,
    );
    assert!(matches!(&request(&outcome).method, Method::GitDiff(params)
        if params.target == "term_original" && params.path == "second.rs"));
    state.handle_endpoint_result("boot-1", &request(&outcome).id, Ok(diff()));
    let mut refresh = ClientShellInput::default();
    state.route_git_view_key(
        &crate::input::TerminalKey::new(KeyCode::Char('r'), KeyModifiers::empty()),
        &mut refresh,
    );
    assert!(
        matches!(&request(&refresh).method, Method::GitStatus(params) if params.target == "term_original")
    );
}

#[test]
fn git_view_accepts_status_from_servers_without_terminal_identity() {
    let mut value = serde_json::to_value(status()).unwrap();
    value["status"]
        .as_object_mut()
        .unwrap()
        .remove("terminal_id");
    let result = serde_json::from_value(value).unwrap();
    let mut state = loaded_state(result);
    let mut refresh = ClientShellInput::default();
    state.route_git_view_key(
        &crate::input::TerminalKey::new(KeyCode::Char('r'), KeyModifiers::empty()),
        &mut refresh,
    );
    assert!(
        matches!(&request(&refresh).method, Method::GitStatus(params) if params.target == "pane_1")
    );
}

#[test]
fn git_view_preserves_distinct_file_names_in_narrow_terminal() {
    let ResponseResult::GitStatus { mut status } = status() else {
        unreachable!()
    };
    for file in &mut status.files {
        file.path = format!("docs/next/website/src/content/docs/{}", file.path);
    }
    let mut state = loaded_state(ResponseResult::GitStatus { status });
    let frame = state.compose(80, 24).unwrap();
    for (rect, index) in &state.hits.git_files {
        let cells =
            &frame.cells[usize::from(rect.y) * usize::from(frame.width) + usize::from(rect.x)..];
        let label = cells
            .iter()
            .take(usize::from(rect.width))
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        assert!(
            label.contains(["first.rs", "second.rs", "third.rs"][*index]),
            "{label}"
        );
        assert!(label.contains('…'));
    }
    assert_eq!(state.hits.git_files.len(), 3);
}

#[test]
fn git_view_passive_mouse_and_unchanged_scroll_do_not_repaint() {
    let mut state = loaded_state(status());
    state.compose(80, 24).unwrap();
    for kind in [
        MouseEventKind::Moved,
        MouseEventKind::Up(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Left),
        MouseEventKind::Down(MouseButton::Right),
    ] {
        let outcome = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
            kind,
            column: 40,
            row: 10,
            modifiers: KeyModifiers::empty(),
        })]);
        assert!(!outcome.repaint, "{kind:?}");
        assert!(outcome.actions.is_empty());
        assert!(outcome.requests.is_empty());
    }
    let area = state.hits.git_diff;
    let mouse = |kind| {
        RawInputEvent::Mouse(MouseEvent {
            kind,
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::empty(),
        })
    };
    assert!(
        !state
            .handle_raw_events(vec![mouse(MouseEventKind::ScrollUp)])
            .repaint
    );
    assert!(
        state
            .handle_raw_events(vec![mouse(MouseEventKind::ScrollDown)])
            .repaint
    );
    assert!(
        !state
            .handle_raw_events(vec![mouse(MouseEventKind::ScrollDown)])
            .repaint
    );
    assert!(
        state
            .handle_raw_events(vec![mouse(MouseEventKind::ScrollUp)])
            .repaint
    );
}
