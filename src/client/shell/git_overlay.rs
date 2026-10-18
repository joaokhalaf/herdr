use super::*;
use crate::api::schema::GitSection;
use crate::client::shell::git_view::ClientGitOverlay;

fn clean(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_control() { '�' } else { ch })
        .collect()
}

fn path_label(path: &str, width: u16) -> String {
    use unicode_segmentation::UnicodeSegmentation;

    let path = clean(path);
    if UnicodeWidthStr::width(path.as_str()) <= usize::from(width) {
        return path;
    }
    if width == 0 {
        return String::new();
    }
    let mut used = 1;
    let mut start = path.len();
    for (index, grapheme) in path.grapheme_indices(true).rev() {
        let columns = UnicodeWidthStr::width(grapheme);
        if used + columns > usize::from(width) {
            break;
        }
        used += columns;
        start = index;
    }
    format!("…{}", &path[start..])
}

pub(super) fn render_git_overlay(
    b: &mut Buffer,
    view: &ClientGitOverlay,
    p: &Palette,
) -> Option<OverlayRender> {
    let area = popup(
        b.area,
        b.area.width.saturating_sub(4),
        b.area.height.saturating_sub(2),
    )?;
    let inner = panel(b, area, p.accent, p.panel_bg)?;
    let text = Style::default().fg(p.text).bg(p.panel_bg);
    let muted = text.fg(p.overlay0);
    let mut rendered = OverlayRender {
        area,
        ..OverlayRender::default()
    };
    rendered.cancel = Rect::new(
        inner.right().saturating_sub(12).max(inner.x),
        inner.y,
        inner.width.min(12),
        1,
    );
    put_text(
        b,
        inner.x,
        inner.y,
        inner.width.saturating_sub(13),
        &format!(" Git changes · {}", clean(&view.label)),
        text.add_modifier(Modifier::BOLD),
    );
    button(b, rendered.cancel, " esc close ", text.bg(p.surface0));
    if inner.width < 64 || inner.height < 14 {
        put_text(
            b,
            inner.x,
            inner.y + 1,
            inner.width,
            " Enlarge terminal to view Git changes",
            muted,
        );
        return Some(rendered);
    }
    if let Some(status) = &view.status {
        put_text(
            b,
            inner.x,
            inner.y + 2,
            inner.width,
            &format!(" {} · {}", clean(&status.branch), clean(&status.root)),
            text.fg(p.accent),
        );
        let added: u64 = status.files.iter().filter_map(|file| file.added).sum();
        let removed: u64 = status.files.iter().filter_map(|file| file.removed).sum();
        put_text(
            b,
            inner.x,
            inner.y + 3,
            inner.width,
            &format!(
                " {} files   +{added} −{removed} (tracked text)",
                status.files.len()
            ),
            muted,
        );
    }
    for (offset, width, label, section) in [
        (1, 19, "1 local changes", GitSection::Local),
        (22, 12, "2 staged", GitSection::Staged),
    ] {
        let rect = Rect::new(inner.x + offset, inner.y + 5, width, 1);
        button(
            b,
            rect,
            label,
            if view.section == section {
                text.bg(p.surface0).fg(p.accent)
            } else {
                muted
            },
        );
        rendered.git_sections.push((rect, section));
    }
    rendered.primary = Rect::new(inner.right() - 12, inner.y + 5, 12, 1);
    button(b, rendered.primary, " r refresh ", text.bg(p.surface0));
    let split = inner.x + (inner.width / 3).clamp(24, 44);
    let top = inner.y + 7;
    let bottom = inner.bottom() - 4;
    for y in top..bottom {
        put_text(b, split, y, 1, "│", muted);
    }
    let rows = usize::from(bottom.saturating_sub(top + 1));
    rendered.git_diff = Rect::new(
        split + 2,
        top + 1,
        inner.right().saturating_sub(split + 3),
        rows as u16,
    );
    put_text(b, inner.x + 1, top, split - inner.x - 2, "FILES", muted);
    if let Some(status) = &view.status {
        let start = view.selected.saturating_sub(rows.saturating_sub(1));
        for (index, file) in status.files.iter().enumerate().skip(start).take(rows) {
            let rect = Rect::new(
                inner.x + 1,
                top + 1 + (index - start) as u16,
                split - inner.x - 2,
                1,
            );
            let selected = index == view.selected;
            let style = if selected { text.bg(p.surface0) } else { text };
            b.set_style(rect, style);
            let color = match file.status.as_str() {
                "D" => p.red,
                "A" | "?" => p.green,
                _ => p.yellow,
            };
            put_text(b, rect.x, rect.y, 2, &file.status, style.fg(color));
            put_text(
                b,
                rect.x + 2,
                rect.y,
                rect.width.saturating_sub(2),
                &path_label(&file.path, rect.width.saturating_sub(2)),
                style,
            );
            rendered.git_files.push((rect, index));
        }
        if let Some(file) = status.files.get(view.selected) {
            put_text(
                b,
                split + 2,
                top,
                inner.right() - split - 3,
                &path_label(&file.path, inner.right() - split - 3),
                muted,
            );
        } else {
            put_text(
                b,
                split + 2,
                top + 1,
                inner.right() - split - 3,
                "No changes in this section",
                text.fg(p.green),
            );
        }
    }
    if view.loading {
        put_text(
            b,
            split + 2,
            top + 1,
            inner.right() - split - 3,
            "Loading Git changes…",
            muted,
        );
    } else {
        for (row, line) in view.lines.iter().skip(view.scroll).take(rows).enumerate() {
            let color = if line.starts_with('+') {
                p.green
            } else if line.starts_with('-') {
                p.red
            } else if line.starts_with("@@") {
                p.accent
            } else {
                p.text
            };
            put_text(
                b,
                split + 2,
                top + 1 + row as u16,
                inner.right() - split - 3,
                line,
                text.fg(color),
            );
        }
    }
    put_text(
        b,
        inner.x + 1,
        inner.bottom() - 3,
        inner.width - 2,
        &clean(
            view.error
                .as_deref()
                .unwrap_or("Changes are shared by all agents using this checkout."),
        ),
        if view.error.is_some() {
            text.fg(p.red)
        } else {
            muted
        },
    );
    put_text(
        b,
        inner.x + 1,
        inner.bottom() - 1,
        inner.width - 2,
        "↑↓ files · PgUp/PgDn diff · tab section · r refresh · esc close",
        muted,
    );
    Some(rendered)
}

#[cfg(test)]
mod tests {
    use super::path_label;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn git_path_label_preserves_unicode_graphemes_and_terminal_width() {
        for path in [
            "deep/path/日本語.rs",
            "deep/path/cafe\u{301}.rs",
            "deep/path/👩‍💻.rs",
        ] {
            for width in 0..32 {
                let label = path_label(path, width);
                assert!(
                    UnicodeWidthStr::width(label.as_str()) <= usize::from(width),
                    "{label:?}"
                );
                assert!(!label.starts_with("…\u{301}"));
                assert!(!label.starts_with("…\u{200d}"));
                if UnicodeWidthStr::width(path) <= usize::from(width) {
                    assert_eq!(label, path);
                }
            }
        }
    }
}
