use std::path::Path;

use crate::app::App;

impl App {
    /// Run through the existing PTY command path: the API does not wait for an
    /// installation to finish, and its output/exit status stay in the setup tab.
    pub(super) fn start_worktree_preparation_command(
        &mut self,
        ws_idx: usize,
        checkout: &Path,
        command: &[String],
    ) -> std::io::Result<()> {
        let (rows, cols) = self.state.new_pane_size(crate::ui::NewPanePlacement::Alone);
        let workspace = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .ok_or_else(|| std::io::Error::other("worktree workspace disappeared"))?;
        let argv = crate::platform::worktree_preparation_argv(command);
        let (tab_idx, terminal, runtime) = workspace.create_tab_argv_command(
            rows,
            cols,
            checkout.to_path_buf(),
            &argv,
            vec![
                ("HERDR_WORKTREE_PATH".into(), checkout.display().to_string()),
                (
                    "HERDR_WORKTREE_SHELL".into(),
                    crate::pane::pane_shell(&self.state.default_shell),
                ),
            ],
            self.state.pane_scrollback_limit_bytes,
            self.state.host_terminal_theme,
            self.state.host_terminal_appearance,
        )?;
        workspace.tabs[tab_idx].set_custom_name("setup".into());
        let root_pane = workspace.tabs[tab_idx].root_pane;
        self.terminal_runtimes.insert(terminal.id.clone(), runtime);
        self.state.terminals.insert(terminal.id.clone(), terminal);
        self.state.remove_alias_shadowed_by_new_pane(root_pane);
        self.state.mark_session_dirty();
        self.emit_tab_created_events(ws_idx, tab_idx);
        Ok(())
    }
}
