use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use crate::api::schema::{Method, Request, ResponseResult};
use crate::app::App;

use super::responses::{encode_error, encode_success};

impl App {
    pub(crate) fn start_git_api_read(
        &self,
        request: Request,
        respond_to: std::sync::mpsc::Sender<String>,
    ) {
        static READS: LazyLock<Arc<tokio::sync::Semaphore>> =
            LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(4)));
        let target = match &request.method {
            Method::GitStatus(params) => &params.target,
            Method::GitDiff(params) => &params.target,
            _ => return,
        };
        let resolved = match self.resolve_terminal_target(target) {
            Ok(resolved) => resolved,
            Err(error) => {
                let body = self.agent_target_error_body(error);
                let _ = respond_to.send(encode_error(request.id, &body.code, body.message));
                return;
            }
        };
        let tab = &self.state.workspaces[resolved.ws_idx].tabs[resolved.tab_idx];
        let cwd = tab
            .foreground_cwd_for_pane(resolved.pane_id, &self.terminal_runtimes)
            .or_else(|| {
                tab.cwd_for_pane(
                    resolved.pane_id,
                    &self.state.terminals,
                    &self.terminal_runtimes,
                )
            });
        let Some(cwd) = cwd else {
            let _ = respond_to.send(encode_error(
                request.id,
                "cwd_unavailable",
                "The terminal has no working directory",
            ));
            return;
        };
        let Ok(permit) = READS.clone().try_acquire_owned() else {
            let _ = respond_to.send(encode_error(
                request.id,
                "git_busy",
                "Git reads are busy; try again shortly",
            ));
            return;
        };
        let terminal_id = resolved.terminal_id;
        tokio::spawn(async move {
            let _permit = permit;
            let result = tokio::time::timeout(
                Duration::from_secs(8),
                read(cwd, request.method, terminal_id),
            )
            .await;
            let response = match result {
                Ok(Ok(result)) => encode_success(request.id, result),
                Ok(Err(error)) => encode_error(request.id, "git_read_failed", error),
                Err(_) => encode_error(
                    request.id,
                    "git_timeout",
                    "Git read timed out; refresh to retry",
                ),
            };
            let _ = respond_to.send(response);
        });
    }
}

async fn read(cwd: PathBuf, method: Method, terminal_id: String) -> Result<ResponseResult, String> {
    match method {
        Method::GitStatus(params) => {
            crate::git_changes::status(&cwd, params.section)
                .await
                .map(|mut status| {
                    status.terminal_id = Some(terminal_id);
                    ResponseResult::GitStatus { status }
                })
        }
        Method::GitDiff(params) => crate::git_changes::diff(&cwd, &params)
            .await
            .map(|diff| ResponseResult::GitDiff { diff }),
        _ => Err("Not a Git read".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::{
        GitDiffParams, GitSection, GitStatusParams, PaneMoveDestination, PaneMoveParams,
        SplitDirection, SuccessResponse,
    };

    #[tokio::test]
    async fn git_api_read_uses_terminal_checkout_instead_of_workspace_identity() {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            rx,
            crate::api::EventHub::default(),
        );
        let root = std::env::temp_dir().join(format!(
            "herdr-git-api-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "--quiet"])
            .output()
            .unwrap();
        assert!(output.status.success());
        std::fs::write(root.join("probe.txt"), "preview\n").unwrap();
        let mut workspace = crate::workspace::Workspace::test_new("git-view");
        workspace.identity_cwd = PathBuf::from("/stale-workspace-identity");
        app.state.workspaces = vec![workspace];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        let pane = app.state.workspaces[0].focused_pane_id().unwrap();
        let target = app.public_pane_id(0, pane).unwrap();
        let terminal = app.state.workspaces[0]
            .pane_state(pane)
            .unwrap()
            .attached_terminal_id
            .clone();
        app.state.terminals.get_mut(&terminal).unwrap().cwd = root.clone();
        app.state
            .workspaces
            .push(crate::workspace::Workspace::test_new("destination"));
        app.state.ensure_test_terminals();
        let destination_tab = app.public_tab_id(1, 0).unwrap();
        app.state.assert_invariants_for_test();
        let (tx, rx) = std::sync::mpsc::channel();
        app.start_git_api_read(
            Request {
                id: "git-read".into(),
                method: Method::GitStatus(GitStatusParams {
                    target: target.clone(),
                    section: GitSection::Local,
                }),
            },
            tx,
        );
        // Move while the initial Git read is in flight. Its captured terminal
        // must remain usable even after the source public pane ID disappears.
        let moved = app.handle_api_request(Request {
            id: "move".into(),
            method: Method::PaneMove(PaneMoveParams {
                pane_id: target.clone(),
                destination: PaneMoveDestination::Tab {
                    tab_id: destination_tab,
                    target_pane_id: None,
                    split: SplitDirection::Down,
                    ratio: None,
                },
                focus: false,
            }),
        });
        let moved: SuccessResponse = serde_json::from_str(&moved).unwrap();
        assert!(
            matches!(moved.result, ResponseResult::PaneMove { move_result } if move_result.changed)
        );
        assert!(app.resolve_terminal_target(&target).is_err());
        app.state.assert_invariants_for_test();
        let response =
            tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(10)).unwrap())
                .await
                .unwrap();
        let response: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::GitStatus { status } = response.result else {
            panic!("Git status response");
        };
        assert_eq!(status.terminal_id.as_deref(), Some(terminal.as_str()));
        assert_eq!(
            PathBuf::from(&status.root).canonicalize().unwrap(),
            root.canonicalize().unwrap()
        );
        let (tx, rx) = std::sync::mpsc::channel();
        app.start_git_api_read(
            Request {
                id: "diff-after-move".into(),
                method: Method::GitDiff(GitDiffParams {
                    target: status.terminal_id.unwrap(),
                    root: status.root,
                    path: "probe.txt".into(),
                    section: GitSection::Local,
                }),
            },
            tx,
        );
        let response =
            tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(10)).unwrap())
                .await
                .unwrap();
        let response: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert!(
            matches!(response.result, ResponseResult::GitDiff { diff } if diff.text.contains("+preview"))
        );
        app.state.assert_invariants_for_test();
        std::fs::remove_dir_all(root).unwrap();
    }
}
