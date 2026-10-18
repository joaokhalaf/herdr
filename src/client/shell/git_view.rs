use super::*;
use crate::api::schema::{
    GitDiffParams, GitSection, GitStatusParams, GitStatusResult, Method, ResponseResult,
};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

#[derive(Debug)]
pub(super) struct ClientGitOverlay {
    pub(super) endpoint: ClientEndpointId,
    pub(super) target: String,
    pub(super) label: String,
    pub(super) token: u64,
    pub(super) section: GitSection,
    pub(super) status: Option<GitStatusResult>,
    pub(super) selected: usize,
    pub(super) lines: Vec<String>,
    pub(super) scroll: usize,
    pub(super) loading: bool,
    pub(super) refresh_pending: bool,
    pub(super) requested_path: Option<String>,
    pub(super) error: Option<String>,
}

impl ClientShellState {
    pub(super) fn open_git_view(&mut self, target: String, outcome: &mut ClientShellInput) {
        let advertised = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == self.active_endpoint_id)
            .and_then(|endpoint| endpoint.methods.as_ref())
            .is_some_and(|methods| methods.contains("git.status") && methods.contains("git.diff"));
        if !advertised {
            outcome.repaint |= self.push_endpoint_notice(
                ClientEndpointNoticeKind::Unsupported,
                "git.status",
                "Git view unavailable",
                "Update and restart this server to enable Git changes.",
            );
            return;
        }
        let label = self
            .snapshot
            .as_deref()
            .and_then(|snapshot| {
                snapshot
                    .agents
                    .iter()
                    .find(|agent| agent.pane_id == target)
                    .map(|agent| agent.name.clone().or(agent.agent.clone()))
            })
            .flatten()
            .unwrap_or_else(|| target.clone());
        self.overlay = Some(ClientShellOverlay::GitChanges(ClientGitOverlay {
            endpoint: self.active_endpoint_id.clone(),
            target,
            label,
            token: 0,
            section: GitSection::Local,
            status: None,
            selected: 0,
            lines: Vec::new(),
            scroll: 0,
            loading: false,
            refresh_pending: false,
            requested_path: None,
            error: None,
        }));
        self.refresh_git_view(outcome);
    }

    fn refresh_git_view(&mut self, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::GitChanges(view)) = self.overlay.as_mut() else {
            return;
        };
        if view.endpoint != self.active_endpoint_id {
            return;
        }
        if view.loading {
            view.refresh_pending = true;
            view.status = None;
            view.lines.clear();
            outcome.repaint = true;
            return;
        }
        view.token = self.next_request_id;
        view.loading = true;
        view.requested_path = None;
        view.error = None;
        view.lines.clear();
        view.status = None;
        view.selected = 0;
        view.scroll = 0;
        let token = view.token;
        let method = Method::GitStatus(GitStatusParams {
            target: view.target.clone(),
            section: view.section,
        });
        if !self.push_endpoint_method_with_kind(
            method,
            PendingEndpointKind::GitRead { token },
            outcome,
        ) {
            self.git_send_failed();
        }
        outcome.repaint = true;
    }

    fn git_send_failed(&mut self) {
        if let Some(ClientShellOverlay::GitChanges(view)) = self.overlay.as_mut() {
            view.loading = false;
            view.error = Some("Server unavailable; refresh to retry".into());
        }
    }

    fn select_git_file(&mut self, selected: usize, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::GitChanges(view)) = self.overlay.as_mut() else {
            return;
        };
        let Some(status) = view.status.as_ref() else {
            return;
        };
        let Some(file) = status.files.get(selected) else {
            return;
        };
        if view.selected == selected
            && (view.loading || !view.lines.is_empty())
            && view.error.is_none()
        {
            return;
        }
        view.selected = selected;
        view.scroll = 0;
        view.lines.clear();
        view.error = None;
        if view.loading {
            outcome.repaint = true;
            return;
        }
        view.loading = true;
        view.requested_path = Some(file.path.clone());
        view.token = self.next_request_id;
        let token = view.token;
        let method = Method::GitDiff(GitDiffParams {
            target: view.target.clone(),
            root: status.root.clone(),
            path: file.path.clone(),
            section: view.section,
        });
        if !self.push_endpoint_method_with_kind(
            method,
            PendingEndpointKind::GitRead { token },
            outcome,
        ) {
            self.git_send_failed();
        }
        outcome.repaint = true;
    }

    pub(super) fn complete_git_read(
        &mut self,
        token: u64,
        result: Result<ResponseResult, ClientShellEndpointError>,
        outcome: &mut ClientShellInput,
    ) {
        let Some(ClientShellOverlay::GitChanges(view)) = self.overlay.as_mut() else {
            return;
        };
        if view.endpoint != self.active_endpoint_id || view.token != token {
            return;
        }
        view.loading = false;
        if let Err(error) = &result {
            if error.code.as_deref() == Some("endpoint_cancelled") {
                view.refresh_pending = false;
                view.error = Some(error.message.clone());
                outcome.repaint = true;
                return;
            }
        }
        // Pin the captured terminal even when a newer section/refresh superseded
        // this status result. Public pane IDs change across workspace moves.
        if let Ok(ResponseResult::GitStatus { status }) = &result {
            if let Some(terminal_id) = &status.terminal_id {
                view.target = terminal_id.clone();
            }
        }
        if view.refresh_pending {
            view.refresh_pending = false;
            self.refresh_git_view(outcome);
            return;
        }
        let desired_path = view
            .status
            .as_ref()
            .and_then(|status| status.files.get(view.selected))
            .map(|file| &file.path);
        if view.requested_path.is_some() && desired_path != view.requested_path.as_ref() {
            let selected = view.selected;
            self.select_git_file(selected, outcome);
            return;
        }
        match result {
            Ok(ResponseResult::GitStatus { status }) if status.section == view.section => {
                let has_files = !status.files.is_empty();
                view.status = Some(status);
                if has_files {
                    self.select_git_file(0, outcome);
                }
            }
            Ok(ResponseResult::GitDiff { diff }) => {
                view.lines = diff
                    .text
                    .lines()
                    .map(|line| {
                        line.chars()
                            .map(|ch| {
                                if ch == '\t' {
                                    ' '
                                } else if ch.is_control() {
                                    '�'
                                } else {
                                    ch
                                }
                            })
                            .collect()
                    })
                    .collect();
                if diff.truncated {
                    view.lines.push("[Preview truncated at 256 KiB]".into());
                }
                if view.lines.is_empty() {
                    view.lines.push("No textual changes".into());
                }
            }
            Err(error) => view.error = Some(error.message),
            _ => view.error = Some("Unexpected Git response; refresh to retry".into()),
        }
        outcome.repaint = true;
    }

    fn set_git_section(&mut self, section: GitSection, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::GitChanges(view)) = self.overlay.as_mut() else {
            return;
        };
        if view.section != section {
            view.section = section;
            self.refresh_git_view(outcome);
        }
    }

    pub(super) fn route_git_view_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some(ClientShellOverlay::GitChanges(view)) = self.overlay.as_mut() else {
            return false;
        };
        let (code, modifiers) = crate::config::normalize_key_combo((key.code, key.modifiers));
        if !modifiers.is_empty() && code != KeyCode::BackTab {
            return true;
        }
        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlay = None;
                outcome.repaint = true;
            }
            KeyCode::Char('r') => self.refresh_git_view(outcome),
            KeyCode::Tab | KeyCode::BackTab => {
                let section = if view.section == GitSection::Local {
                    GitSection::Staged
                } else {
                    GitSection::Local
                };
                self.set_git_section(section, outcome);
            }
            KeyCode::Char('1') => self.set_git_section(GitSection::Local, outcome),
            KeyCode::Char('2') => self.set_git_section(GitSection::Staged, outcome),
            KeyCode::Down | KeyCode::Char('j') => {
                let last = view
                    .status
                    .as_ref()
                    .map_or(0, |status| status.files.len().saturating_sub(1));
                let next = view.selected.saturating_add(1).min(last);
                self.select_git_file(next, outcome);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let next = view.selected.saturating_sub(1);
                self.select_git_file(next, outcome);
            }
            KeyCode::PageDown => {
                let next = view
                    .scroll
                    .saturating_add(10)
                    .min(view.lines.len().saturating_sub(1));
                outcome.repaint |= next != view.scroll;
                view.scroll = next;
            }
            KeyCode::PageUp => {
                let next = view.scroll.saturating_sub(10);
                outcome.repaint |= next != view.scroll;
                view.scroll = next;
            }
            _ => {}
        }
        true
    }

    pub(super) fn route_git_view_mouse(
        &mut self,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) -> bool {
        if !matches!(self.overlay, Some(ClientShellOverlay::GitChanges(_))) {
            return false;
        }
        let point = (mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if contains(self.hits.overlay_cancel, point) {
                    self.overlay = None;
                    outcome.repaint = true;
                } else if contains(self.hits.overlay_primary, point) {
                    self.refresh_git_view(outcome);
                } else if let Some((_, section)) = self
                    .hits
                    .git_sections
                    .iter()
                    .find(|(rect, _)| contains(*rect, point))
                {
                    self.set_git_section(*section, outcome);
                } else if let Some((_, selected)) = self
                    .hits
                    .git_files
                    .iter()
                    .find(|(rect, _)| contains(*rect, point))
                {
                    self.select_git_file(*selected, outcome);
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                if contains(self.hits.git_diff, point) =>
            {
                if let Some(ClientShellOverlay::GitChanges(view)) = self.overlay.as_mut() {
                    let next = if mouse.kind == MouseEventKind::ScrollUp {
                        view.scroll.saturating_sub(3)
                    } else {
                        view.scroll
                            .saturating_add(3)
                            .min(view.lines.len().saturating_sub(1))
                    };
                    outcome.repaint |= view.scroll != next;
                    view.scroll = next;
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                if let Some(ClientShellOverlay::GitChanges(view)) = self.overlay.as_ref() {
                    let last = view
                        .status
                        .as_ref()
                        .map_or(0, |status| status.files.len().saturating_sub(1));
                    let next = if mouse.kind == MouseEventKind::ScrollUp {
                        view.selected.saturating_sub(1)
                    } else {
                        view.selected.saturating_add(1).min(last)
                    };
                    self.select_git_file(next, outcome);
                }
            }
            _ => {}
        }
        true
    }
}
