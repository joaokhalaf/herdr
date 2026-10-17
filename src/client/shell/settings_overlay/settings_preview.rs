use super::*;

/// A schematic of the selected action, built only while settings are visible.
/// It never changes live geometry or saves the previewed configuration.
pub(super) fn render_layout_preview(
    buffer: &mut Buffer,
    area: Rect,
    settings: &ClientSettingsOverlay,
    config: &ClientShellConfig,
    can_edit_borders: bool,
) {
    if area.width < 22 || area.height < 9 {
        return;
    }
    let palette = &config.palette;
    let text = Style::default().fg(palette.text).bg(palette.panel_bg);
    let muted = text.fg(palette.overlay1);
    let mut width = settings.sidebar_width;
    let mut agents = config.show_sidebar_agents;
    let mut spaces = config.show_sidebar_spaces;
    let mut agent_gap = config.agents.row_gap;
    let mut space_gap = config.spaces.row_gap;
    let mut borders = config.pane_borders;
    let mut outer = config.pane_outer_borders;
    if settings.section == ClientSettingsSection::Sidebar {
        let (min, max) = crate::config::validated_sidebar_bounds(
            config.sidebar_min_width,
            config.sidebar_max_width,
        )
        .unwrap_or((18, 36));
        width = width.clamp(min, max);
        match settings.selected {
            0 => width = width.saturating_sub(2).max(min),
            1 => width = width.saturating_add(2).min(max),
            2 => agents = !agents,
            3 => spaces = !spaces,
            _ => {
                width = 26u16.clamp(min, max);
                agents = true;
                spaces = true;
            }
        }
    } else {
        match settings.selected {
            0 if can_edit_borders => {
                borders = match borders {
                    crate::config::PaneBordersConfig::Auto => {
                        crate::config::PaneBordersConfig::Always
                    }
                    crate::config::PaneBordersConfig::Always => {
                        crate::config::PaneBordersConfig::Off
                    }
                    crate::config::PaneBordersConfig::Off => crate::config::PaneBordersConfig::Auto,
                };
            }
            1 if can_edit_borders => outer = !outer,
            2..=4 => {
                agent_gap = settings.selected as u16 - 2;
                space_gap = agent_gap;
            }
            5 => {
                agent_gap = 0;
                space_gap = 0;
                if can_edit_borders {
                    borders = crate::config::PaneBordersConfig::Auto;
                    outer = true;
                }
            }
            _ => {}
        }
    }
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        "preview · selected change",
        text.add_modifier(Modifier::BOLD),
    );
    let frame = Rect::new(
        area.x,
        area.y + 2,
        area.width,
        area.height.saturating_sub(2).min(17),
    );
    let Some(inside) = panel(buffer, frame, palette.surface0, palette.panel_bg) else {
        return;
    };
    let sidebar_width = if agents || spaces {
        // Keep text legible at schematic scale, while reflecting width changes.
        (12 + width.saturating_sub(18) / 2).min(inside.width.saturating_sub(9))
    } else {
        0
    };
    if sidebar_width > 0 {
        let sidebar = Rect::new(
            inside.x,
            inside.y,
            sidebar_width.saturating_sub(1),
            inside.height,
        );
        let mut y = sidebar.y;
        let mut line = |label: &str, style: Style, advance: u16| {
            if y < sidebar.bottom() {
                put_text(buffer, sidebar.x, y, sidebar.width, label, style);
            }
            y = y.saturating_add(advance);
        };
        if spaces {
            line("spaces", muted, 1);
            line("▾ my-app", text, 1);
            line(
                "  └ feat/login",
                text.fg(palette.accent),
                1 + space_gap.min(2),
            );
            line("other-app", text, 2);
        }
        if agents {
            line("agents", muted, 1);
            line("● Claude", text.fg(palette.accent), 1 + agent_gap.min(2));
            line("○ Codex", text, 1);
        }
        let x = inside.x + sidebar_width - 1;
        for y in inside.y..inside.bottom() {
            put_text(buffer, x, y, 1, "│", muted);
        }
    }
    let panes = Rect::new(
        inside.x + sidebar_width,
        inside.y,
        inside.width - sidebar_width,
        inside.height,
    );
    let bordered = borders.shows_borders(true);
    let panes = if bordered && outer {
        panel(buffer, panes, palette.accent, palette.panel_bg).unwrap_or(panes)
    } else {
        panes
    };
    let middle = panes.y + panes.height / 2;
    if bordered {
        for x in panes.x..panes.right() {
            put_text(buffer, x, middle, 1, "─", text.fg(palette.accent));
        }
    }
    put_text(
        buffer,
        panes.x + 1,
        panes.y + 1,
        panes.width.saturating_sub(2),
        "terminal 1",
        muted,
    );
    if middle + 2 < panes.bottom() {
        put_text(
            buffer,
            panes.x + 1,
            middle + 2,
            panes.width.saturating_sub(2),
            "terminal 2",
            muted,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(
        config: &ClientShellConfig,
        section: ClientSettingsSection,
        selected: usize,
    ) -> Buffer {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(
            &crate::config::Config::default(),
        ));
        state.open_settings_overlay();
        let Some(ClientShellOverlay::Settings(mut settings)) = state.overlay.take() else {
            panic!("settings overlay");
        };
        settings.section = section;
        settings.selected = selected;
        let mut buffer = Buffer::empty(Rect::new(0, 0, 48, 22));
        let area = buffer.area;
        render_layout_preview(&mut buffer, area, &settings, config, true);
        buffer
    }

    fn row_with(buffer: &Buffer, label: &str) -> Option<u16> {
        (buffer.area.y..buffer.area.bottom()).find(|y| {
            let row: String = (buffer.area.x..buffer.area.right())
                .map(|x| buffer[(x, *y)].symbol())
                .collect();
            row.contains(label)
        })
    }

    #[test]
    fn spacing_preview_changes_sample_rows_without_applying_configuration() {
        let config = ClientShellConfig::from_config(&crate::config::Config::default());
        let compact = preview(&config, ClientSettingsSection::Appearance, 2);
        let roomy = preview(&config, ClientSettingsSection::Appearance, 4);
        for (buffer, gap) in [(&compact, 0), (&roomy, 2)] {
            let spaces = row_with(buffer, "spaces").unwrap();
            let parent = row_with(buffer, "my-app").unwrap();
            let child = row_with(buffer, "feat/login").unwrap();
            let other = row_with(buffer, "other-app").unwrap();
            let agents = row_with(buffer, "agents").unwrap();
            let claude = row_with(buffer, "Claude").unwrap();
            let codex = row_with(buffer, "Codex").unwrap();
            assert!(spaces < agents);
            assert_eq!(child, parent + 1, "worktree children stay attached");
            assert_eq!(other, child + 1 + gap);
            assert_eq!(codex, claude + 1 + gap);
        }
        assert_eq!(config.agents.row_gap, 0);
        assert_eq!(config.spaces.row_gap, 0);
    }

    #[test]
    fn sidebar_preview_reclaims_space_and_shows_worktree_grouping() {
        let mut config = ClientShellConfig::from_config(&crate::config::Config::default());
        let shown = preview(&config, ClientSettingsSection::Sidebar, 1);
        assert!(row_with(&shown, "Claude").is_some());
        assert!(row_with(&shown, "feat/login").is_some());
        config.show_sidebar_agents = false;
        let hidden = preview(&config, ClientSettingsSection::Sidebar, 3);
        assert!(row_with(&hidden, "agents").is_none());
        assert!(row_with(&hidden, "spaces").is_none());
        assert!(row_with(&hidden, "terminal 1").is_some());
        assert!(
            config.show_sidebar_spaces,
            "preview must not apply the toggle"
        );
        let restored = preview(&config, ClientSettingsSection::Sidebar, 4);
        assert!(row_with(&restored, "Claude").is_some());
        assert!(row_with(&restored, "feat/login").is_some());
    }

    #[test]
    fn responsive_settings_keep_choices_and_preview_separate() {
        let config = ClientShellConfig::from_config(&crate::config::Config::default());
        let mut state = ClientShellState::new(ClientShellConfig::from_config(
            &crate::config::Config::default(),
        ));
        state.open_settings_overlay();
        let Some(ClientShellOverlay::Settings(mut settings)) = state.overlay.take() else {
            panic!("settings overlay");
        };
        for section in [
            ClientSettingsSection::Appearance,
            ClientSettingsSection::Sidebar,
        ] {
            settings.section = section;
            for (cols, rows, shows_preview) in [(110, 32, true), (64, 38, true), (32, 18, false)] {
                let mut buffer = Buffer::empty(Rect::new(0, 0, cols, rows));
                let overlay =
                    render_settings_overlay(&mut buffer, &settings, false, &config, true).unwrap();
                assert_eq!(
                    row_with(&buffer, "preview · selected change").is_some(),
                    shows_preview
                );
                assert!(!overlay.settings_choices.is_empty());
                for (choice, _) in overlay.settings_choices {
                    assert!(choice.right() <= cols && choice.bottom() <= rows);
                    if shows_preview {
                        assert!(
                            !((choice.x..choice.right())
                                .any(|x| buffer[(x, choice.y)].symbol() == "│"))
                        );
                    }
                }
            }
        }
    }
}
