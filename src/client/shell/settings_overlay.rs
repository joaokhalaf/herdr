use super::*;

mod settings_preview;
use settings_preview::render_layout_preview;

fn choice_style(selected: bool, palette: &Palette) -> Style {
    if selected {
        Style::default()
            .fg(contrast(palette))
            .bg(palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.text).bg(palette.panel_bg)
    }
}

fn draw_choice(
    buffer: &mut Buffer,
    rect: Rect,
    label: &str,
    selected: bool,
    current: bool,
    palette: &Palette,
) {
    let style = choice_style(selected, palette);
    buffer.set_style(rect, style);
    let marker = if selected { "▸" } else { " " };
    let current = if current { " ✓" } else { "" };
    put_text(
        buffer,
        rect.x,
        rect.y,
        rect.width,
        &format!(" {marker} {label}{current}"),
        style,
    );
}

pub(super) fn render_settings_overlay(
    buffer: &mut Buffer,
    settings: &ClientSettingsOverlay,
    integration_updates_available: bool,
    config: &ClientShellConfig,
    can_edit_borders: bool,
) -> Option<OverlayRender> {
    let palette = &config.palette;
    let integration_height = 14u16
        .saturating_add(settings.integrations.len().max(1) as u16)
        .saturating_add(settings.integration_messages.len().min(6) as u16);
    let visual = matches!(
        settings.section,
        ClientSettingsSection::Appearance | ClientSettingsSection::Sidebar
    );
    let height = if settings.section == ClientSettingsSection::Integrations {
        integration_height.max(22)
    } else if visual {
        if buffer.area.width < 74 {
            34
        } else {
            28
        }
    } else {
        22
    };
    let popup = popup(buffer.area, if visual { 104 } else { 76 }, height)?;
    let inner = panel(buffer, popup, palette.accent, palette.panel_bg)?;
    if inner.width < 20 || inner.height < 8 {
        return Some(render_small_settings(buffer, popup, inner, palette));
    }

    put_text(
        buffer,
        inner.x,
        inner.y,
        inner.width,
        " settings",
        Style::default()
            .fg(palette.text)
            .bg(palette.panel_bg)
            .add_modifier(Modifier::BOLD),
    );

    let integration_badge = integration_updates_available
        || settings
            .integrations
            .iter()
            .any(|integration| integration.state == crate::api::schema::IntegrationState::Outdated);
    let mut tab_x = inner.x;
    let mut tab_y = inner.y + 1;
    let mut tab_hits = Vec::new();
    for section in ClientSettingsSection::ALL {
        let badge = *section == ClientSettingsSection::Integrations && integration_badge;
        let label = if badge {
            format!(" ● {} ", section.label())
        } else {
            format!(" {} ", section.label())
        };
        if tab_x > inner.x && tab_x.saturating_add(display_width(&label)) > inner.right() {
            tab_x = inner.x;
            tab_y += 1;
        }
        let width = display_width(&label).min(inner.width);
        let rect = Rect::new(tab_x, tab_y, width, 1);
        let active = *section == settings.section;
        let style = if active {
            Style::default()
                .fg(contrast(palette))
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.overlay1).bg(palette.panel_bg)
        };
        buffer.set_style(rect, style);
        put_text(buffer, rect.x, rect.y, rect.width, &label, style);
        if badge && !active {
            put_text(
                buffer,
                rect.x.saturating_add(1),
                rect.y,
                rect.width.saturating_sub(1).min(2),
                "● ",
                Style::default()
                    .fg(palette.accent)
                    .bg(palette.panel_bg)
                    .add_modifier(Modifier::BOLD),
            );
        }
        tab_hits.push((rect, *section));
        tab_x = tab_x.saturating_add(width.saturating_add(1));
    }
    put_text(
        buffer,
        inner.x,
        tab_y + 1,
        inner.width,
        &"─".repeat(inner.width as usize),
        Style::default().fg(palette.surface0).bg(palette.panel_bg),
    );

    let content = Rect::new(
        inner.x,
        tab_y + 3,
        inner.width,
        inner.bottom().saturating_sub(3).saturating_sub(tab_y + 3),
    );
    let mut choice_hits = Vec::new();
    if content.height < 4 {
        return Some(render_small_settings(buffer, popup, inner, palette));
    }
    let (choices_area, preview_area) = if visual && content.width >= 68 {
        let width = content.width / 2;
        (
            Rect::new(content.x, content.y, width, content.height),
            Some(Rect::new(
                content.x + width + 2,
                content.y,
                content.width - width - 2,
                content.height,
            )),
        )
    } else if visual && content.height >= 22 {
        (
            Rect::new(content.x, content.y, content.width, 10),
            Some(Rect::new(
                content.x,
                content.y + 11,
                content.width,
                content.height - 11,
            )),
        )
    } else {
        (content, None)
    };
    match settings.section {
        ClientSettingsSection::Appearance => {
            let borders = match config.pane_borders {
                crate::config::PaneBordersConfig::Auto => "auto",
                crate::config::PaneBordersConfig::Always => "always",
                crate::config::PaneBordersConfig::Off => "off",
            };
            render_choice_section(
                buffer,
                choices_area,
                "borders and list spacing",
                "preview selection; enter applies",
                &[
                    &format!("pane borders: {borders} (cycle)"),
                    if config.pane_outer_borders {
                        "outer borders: shown → hide"
                    } else {
                        "outer borders: hidden → show"
                    },
                    &format!(
                        "compact (0 rows; current: {}/{})",
                        config.agents.row_gap, config.spaces.row_gap
                    ),
                    "comfortable (1 blank row)",
                    "roomy (2 blank rows)",
                    "restore default borders and spacing",
                ],
                settings.selected,
                palette,
                &mut choice_hits,
            );
        }
        ClientSettingsSection::Theme => {
            let visible = usize::from(content.height);
            let scroll = settings.selected.saturating_sub(visible.saturating_sub(1));
            for (visible_index, (index, name)) in crate::config::THEME_NAMES
                .iter()
                .enumerate()
                .skip(scroll)
                .take(visible)
                .enumerate()
            {
                let rect = Rect::new(
                    content.x,
                    content.y + visible_index as u16,
                    content.width,
                    1,
                );
                draw_choice(
                    buffer,
                    rect,
                    name,
                    index == settings.selected,
                    super::super::settings::normalized_theme_name(name)
                        == super::super::settings::normalized_theme_name(
                            &settings.original_theme_name,
                        ),
                    palette,
                );
                choice_hits.push((rect, index));
            }
        }
        ClientSettingsSection::Indicators => {
            render_choice_section(
                buffer,
                content,
                "agent status indicators",
                "choose color dots or distinct symbols for each state",
                &["color dots  ● ● ● ○ ·", "distinct symbols  × ◐ ✓ ○ ·"],
                settings.selected,
                palette,
                &mut choice_hits,
            );
        }
        ClientSettingsSection::Sidebar => {
            render_choice_section(
                buffer,
                choices_area,
                "sidebar layout",
                "preview selection; enter or click applies",
                &[
                    &format!("narrower (current: {} columns)", settings.sidebar_width),
                    &format!("wider (current: {} columns)", settings.sidebar_width),
                    if config.show_sidebar_agents {
                        "agents: shown → hide"
                    } else {
                        "agents: hidden → show"
                    },
                    if config.show_sidebar_spaces {
                        "spaces / worktrees: shown → hide"
                    } else {
                        "spaces / worktrees: hidden → show"
                    },
                    "restore default sidebar",
                ],
                settings.selected,
                palette,
                &mut choice_hits,
            );
        }
        ClientSettingsSection::Sound => {
            render_choice_section(
                buffer,
                content,
                "sound alerts",
                "play sounds when agents change state in background",
                &["on", "off"],
                settings.selected,
                palette,
                &mut choice_hits,
            );
        }
        ClientSettingsSection::Toast => {
            render_choice_section(
                buffer,
                content,
                "notification popups",
                "choose where background popup notifications should appear",
                &["off", "inside herdr", "via terminal", "via system"],
                settings.selected,
                palette,
                &mut choice_hits,
            );
        }
        ClientSettingsSection::Integrations => {
            render_integrations(buffer, content, settings, palette);
        }
    }

    if let Some(area) = preview_area {
        render_layout_preview(buffer, area, settings, config, can_edit_borders);
    }

    let installable = settings
        .integrations
        .iter()
        .any(super::super::settings::integration_needs_install);
    let show_primary = settings.section != ClientSettingsSection::Integrations || installable;
    let labels = if show_primary { vec![10, 12] } else { vec![12] };
    let buttons = row(inner, &labels, 2, inner.height.saturating_sub(1));
    let (primary, close) = if show_primary {
        let primary = buttons[0];
        button(
            buffer,
            primary,
            if settings.section == ClientSettingsSection::Integrations {
                " ↵ install "
            } else {
                " ↵ apply "
            },
            Style::default()
                .fg(contrast(palette))
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD),
        );
        (primary, buttons[1])
    } else {
        (Rect::default(), buttons[0])
    };
    button(
        buffer,
        close,
        " esc close ",
        Style::default()
            .fg(palette.text)
            .bg(palette.surface0)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        buffer,
        inner.x,
        inner.bottom().saturating_sub(2),
        inner.width,
        " ↑↓ select  tab section",
        Style::default().fg(palette.overlay1).bg(palette.panel_bg),
    );

    Some(OverlayRender {
        area: popup,
        primary,
        cancel: close,
        settings_popup: popup,
        settings_tabs: tab_hits,
        settings_choices: choice_hits,
        ..OverlayRender::default()
    })
}

fn render_small_settings(
    buffer: &mut Buffer,
    popup: Rect,
    inner: Rect,
    palette: &Palette,
) -> OverlayRender {
    ratatui::widgets::Widget::render(ratatui::widgets::Clear, inner, buffer);
    let style = Style::default().fg(palette.text).bg(palette.panel_bg);
    buffer.set_style(inner, style);
    if !inner.is_empty() {
        put_text(
            buffer,
            inner.x,
            inner.y,
            inner.width,
            " settings: enlarge terminal",
            style,
        );
    }
    let close = Rect::new(
        inner.x,
        inner.bottom().saturating_sub(1),
        inner.width.min(12),
        u16::from(!inner.is_empty()),
    );
    button(buffer, close, " esc close ", style);
    OverlayRender {
        area: popup,
        cancel: close,
        settings_popup: popup,
        ..OverlayRender::default()
    }
}

fn render_choice_section(
    buffer: &mut Buffer,
    area: Rect,
    title: &str,
    description: &str,
    choices: &[&str],
    selected: usize,
    palette: &Palette,
    hits: &mut Vec<(Rect, usize)>,
) {
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        title,
        Style::default()
            .fg(palette.text)
            .bg(palette.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        buffer,
        area.x,
        area.y + 1,
        area.width,
        description,
        Style::default().fg(palette.overlay1).bg(palette.panel_bg),
    );
    let row_gap = u16::from(choices.len() > 2 && usize::from(area.height) >= 2 + choices.len() * 2);
    let visible =
        usize::from(area.height.saturating_sub(3).saturating_add(row_gap) / (1 + row_gap));
    let start = selected.saturating_sub(visible.saturating_sub(1));
    for (offset, (index, choice)) in choices
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .enumerate()
    {
        let y = area.y + 3 + offset as u16 * (1 + row_gap);
        if y >= area.bottom() {
            break;
        }
        let rect = Rect::new(area.x, y, area.width, 1);
        draw_choice(buffer, rect, choice, index == selected, false, palette);
        hits.push((rect, index));
    }
}

fn render_integrations(
    buffer: &mut Buffer,
    area: Rect,
    settings: &ClientSettingsOverlay,
    palette: &Palette,
) {
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        "agent integrations",
        Style::default()
            .fg(palette.text)
            .bg(palette.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        buffer,
        area.x,
        area.y + 1,
        area.width,
        "enable session restore and, where supported, direct status updates",
        Style::default().fg(palette.overlay1).bg(palette.panel_bg),
    );
    if settings.loading_integrations {
        put_text(
            buffer,
            area.x,
            area.y + 3,
            area.width,
            " loading integrations…",
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
        return;
    }
    if settings.integrations.is_empty() {
        put_text(
            buffer,
            area.x,
            area.y + 3,
            area.width,
            " no integration targets available",
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
        return;
    }
    for (index, integration) in settings.integrations.iter().enumerate() {
        let y = area.y + 3 + index as u16;
        if y >= area.bottom() {
            break;
        }
        let (marker, color, status) = match integration.state {
            crate::api::schema::IntegrationState::Current => ("✓", palette.green, "installed"),
            crate::api::schema::IntegrationState::Outdated => {
                ("↻", palette.yellow, "update available")
            }
            crate::api::schema::IntegrationState::NotInstalled if integration.available => {
                ("+", palette.accent, "available")
            }
            crate::api::schema::IntegrationState::NotInstalled => {
                ("–", palette.overlay0, "not found")
            }
        };
        put_text(
            buffer,
            area.x,
            y,
            3,
            &format!(" {marker}"),
            Style::default().fg(color).bg(palette.panel_bg),
        );
        put_text(
            buffer,
            area.x + 3,
            y,
            11.min(area.width.saturating_sub(3)),
            &format!("{:<9}", integration.label),
            Style::default().fg(palette.subtext0).bg(palette.panel_bg),
        );
        put_text(
            buffer,
            area.x + 14,
            y,
            area.width.saturating_sub(14),
            status,
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
    }
    let message_y = area
        .y
        .saturating_add(4)
        .saturating_add(settings.integrations.len() as u16);
    for (offset, message) in settings.integration_messages.iter().take(6).enumerate() {
        let y = message_y.saturating_add(offset as u16);
        if y >= area.bottom() {
            break;
        }
        put_text(
            buffer,
            area.x,
            y,
            area.width,
            &format!(" {message}"),
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
    }
    if settings.installing_integrations && message_y < area.bottom() {
        put_text(
            buffer,
            area.x,
            message_y,
            area.width,
            " installing…",
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
    }
}
