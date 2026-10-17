#[derive(Clone, Copy)]
pub(crate) enum ConfigEdit<'a> {
    Theme(&'a str),
    Appearance {
        borders: Option<(super::PaneBordersConfig, bool)>,
        row_gap: Option<u16>,
    },
    Sidebar {
        width: u16,
        agents: bool,
        spaces: bool,
    },
    StatusIndicators(super::StatusIndicatorStyle),
    Sound(bool),
    ToastDelivery(super::ToastDelivery),
}

impl ConfigEdit<'_> {
    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Theme(_) => "theme",
            Self::Appearance { .. } => "appearance settings",
            Self::Sidebar { .. } => "sidebar settings",
            Self::StatusIndicators(_) => "status indicators",
            Self::Sound(_) => "sound setting",
            Self::ToastDelivery(_) => "toast setting",
        }
    }

    pub(crate) fn apply(self, content: &str) -> String {
        match self {
            Self::Appearance { borders, row_gap } => {
                let mut content = content.to_owned();
                if let Some((mode, outer)) = borders {
                    let mode = match mode {
                        super::PaneBordersConfig::Auto => "auto",
                        super::PaneBordersConfig::Always => "always",
                        super::PaneBordersConfig::Off => "off",
                    };
                    content = super::upsert_section_value(
                        &content,
                        "ui",
                        "pane_borders",
                        &format!("{mode:?}"),
                    );
                    content =
                        super::upsert_section_bool(&content, "ui", "pane_outer_borders", outer);
                }
                if let Some(gap) = row_gap {
                    for section in ["ui.sidebar.agents", "ui.sidebar.spaces"] {
                        content = super::upsert_section_value(
                            &content,
                            section,
                            "row_gap",
                            &gap.to_string(),
                        );
                    }
                }
                content
            }
            Self::Sidebar {
                width,
                agents,
                spaces,
            } => {
                let content =
                    super::upsert_section_value(content, "ui", "sidebar_width", &width.to_string());
                let content =
                    super::upsert_section_bool(&content, "ui", "show_sidebar_agents", agents);
                super::upsert_section_bool(&content, "ui", "show_sidebar_spaces", spaces)
            }
            Self::Theme(name) => {
                let content =
                    super::upsert_section_value(content, "theme", "name", &format!("\"{name}\""));
                super::upsert_section_bool(&content, "theme", "auto_switch", false)
            }
            Self::StatusIndicators(style) => super::upsert_section_value(
                content,
                "ui",
                "status_indicators",
                &format!("\"{}\"", style.as_str()),
            ),
            Self::Sound(enabled) => {
                super::upsert_section_bool(content, "ui.sound", "enabled", enabled)
            }
            Self::ToastDelivery(delivery) => {
                let value = match delivery {
                    super::ToastDelivery::Off => "\"off\"",
                    super::ToastDelivery::Herdr => "\"herdr\"",
                    super::ToastDelivery::Terminal => "\"terminal\"",
                    super::ToastDelivery::System => "\"system\"",
                };
                let content = super::upsert_section_value(content, "ui.toast", "delivery", value);
                super::remove_section_key(&content, "ui.toast", "enabled")
            }
        }
    }
}

impl ConfigEdit<'_> {
    pub(crate) fn requires_server_reload(self) -> bool {
        !matches!(
            self,
            Self::Sidebar { .. } | Self::Appearance { borders: None, .. }
        )
    }
}

pub(crate) fn update_file_at(
    path: &std::path::Path,
    description: &str,
    update: impl FnOnce(&str) -> String,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create config directory: {error}"))?;
    }
    let content = match super::io::read_optional_config(path) {
        Ok(Some(content)) => content,
        Ok(None) => String::new(),
        Err(error) => {
            return Err(format!(
                "failed to read config before saving {description}: {error}"
            ));
        }
    };
    std::fs::write(path, update(&content))
        .map_err(|error| format!("failed to save {description}: {error}"))
}

pub(crate) fn write_edit(edit: ConfigEdit<'_>) -> Result<(), String> {
    update_file_at(&super::config_path(), edit.description(), |content| {
        edit.apply(content)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_edits_preserve_custom_tokens_rows_and_worktree_preparation() {
        let source = "[ui]\naccent = 'red'\nsidebar_width = 31\n[theme.custom]\naccent = '#010203'\ntext = '#112233'\n[theme.custom.light]\naccent = '#ffffff'\n[ui.sidebar.agents]\nrow_gap = 4\nrows = [[{ token = 'machine' }]]\n[ui.sidebar.spaces]\nrow_gap = 3\n[[worktrees.prepare]]\nrepository = '/tmp/project'\ncopy_files = ['.env']\ncommand = ['sh', '-c', 'echo fixture']\n";
        let content = ConfigEdit::Appearance {
            borders: Some((crate::config::PaneBordersConfig::Off, false)),
            row_gap: Some(1),
        }
        .apply(source);
        let config: crate::config::Config = toml::from_str(&content).unwrap();
        let custom = config.theme.custom.unwrap();
        assert_eq!(custom.accent.as_deref(), Some("#010203"));
        assert_eq!(custom.text.as_deref(), Some("#112233"));
        assert_eq!(custom.light.unwrap().accent.as_deref(), Some("#ffffff"));
        assert_eq!(config.ui.accent, "red");
        assert_eq!(config.ui.sidebar_width, 31);
        assert_eq!(
            config.ui.pane_borders,
            crate::config::PaneBordersConfig::Off
        );
        assert!(!config.ui.pane_outer_borders);
        assert_eq!(config.ui.sidebar.agents.row_gap, 1);
        assert_eq!(config.ui.sidebar.spaces.row_gap, 1);
        assert_eq!(config.ui.sidebar.agents.rows.len(), 1);
        assert_eq!(config.worktrees.prepare[0].copy_files, [".env"]);
        assert_eq!(
            config.worktrees.prepare[0].command,
            ["sh", "-c", "echo fixture"]
        );
        assert!(!ConfigEdit::Appearance {
            borders: None,
            row_gap: Some(2)
        }
        .requires_server_reload());
        assert!(ConfigEdit::Appearance {
            borders: Some((crate::config::PaneBordersConfig::Auto, true)),
            row_gap: None
        }
        .requires_server_reload());
    }

    #[test]
    fn sidebar_edit_preserves_other_settings_and_loads_visibility() {
        let content = ConfigEdit::Sidebar { width: 30, agents: false, spaces: true }.apply(
            "[theme]\nname = \"terminal\"\n[ui]\nsidebar_min_width = 20\n[ui.sidebar.agents]\nrow_gap = 1\n",
        );
        let config: crate::config::Config = toml::from_str(&content).unwrap();
        assert_eq!(config.ui.sidebar_width, 30);
        assert_eq!(config.ui.sidebar_min_width, 20);
        assert!(!config.ui.show_sidebar_agents);
        assert!(config.ui.show_sidebar_spaces);
        assert_eq!(config.ui.sidebar.agents.row_gap, 1);
        assert_eq!(config.theme.name.as_deref(), Some("terminal"));
    }

    #[test]
    fn update_file_at_does_not_move_a_leading_bom_into_the_file() {
        let dir = std::env::temp_dir().join(format!("herdr-config-bom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            b"\xEF\xBB\xBF[terminal]\ndefault_shell = \"pwsh.exe\"\n",
        )
        .unwrap();

        update_file_at(&path, "onboarding setting", |content| {
            crate::config::upsert_top_level_bool(content, "onboarding", false)
        })
        .unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);

        assert!(
            !written.contains('\u{feff}'),
            "unexpected BOM in {written:?}"
        );
        assert!(
            toml::from_str::<toml::Value>(&written).is_ok(),
            "written config is not valid TOML: {written:?}"
        );
    }
}
