use std::fs::{self, File, OpenOptions};
use std::path::{Component, Path};

use crate::config::WorktreePreparationConfig;

mod install;

pub(crate) fn command(
    checkout: &Path,
    rule: &WorktreePreparationConfig,
) -> Result<Vec<String>, String> {
    if !rule.command.is_empty() || !rule.install_dependencies {
        return Ok(rule.command.clone());
    }
    install::command(checkout)
}

#[cfg(test)]
mod tests;

pub(crate) fn for_repository(
    source: &Path,
    rules: &[WorktreePreparationConfig],
) -> Result<WorktreePreparationConfig, String> {
    let source = super::canonical_or_original(source);
    let mut matched = None;
    for rule in rules {
        let repository = super::expand_tilde_path(&rule.repository);
        if !repository.is_absolute() {
            return Err("worktree preparation repository must be an absolute path".into());
        }
        if super::canonical_or_original(&repository) != source {
            continue;
        }
        if matched.is_some() {
            return Err("multiple preparation rules match this repository".into());
        }
        for path in &rule.copy_files {
            validate_relative_file(path)?;
        }
        if rule
            .command
            .first()
            .is_some_and(|program| program.trim().is_empty())
            || rule
                .command
                .iter()
                .any(|arg| arg.chars().any(char::is_control))
        {
            return Err(
                "preparation command needs an executable and arguments without control characters"
                    .into(),
            );
        }
        matched = Some(rule.clone());
    }
    Ok(matched.unwrap_or_default())
}

fn validate_relative_file(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.is_empty()
        || path.components().any(|component| match component {
            Component::Normal(name) => name.eq_ignore_ascii_case(".git"),
            _ => true,
        })
    {
        return Err(format!(
            "copy_files requires relative file paths without .git, . or ..: {value}"
        ));
    }
    Ok(())
}

/// Called only by the worktree worker, after Git has created a fresh checkout.
/// Never reads file contents into diagnostics or follows repository symlinks.
pub(crate) fn copy_files(source: &Path, checkout: &Path, files: &[String]) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }
    let source = fs::canonicalize(source).map_err(|err| err.to_string())?;
    let checkout = fs::canonicalize(checkout).map_err(|err| err.to_string())?;
    for relative in files {
        validate_relative_file(relative)?;
        let result = (|| -> std::io::Result<()> {
            let mut from = source.clone();
            let mut to = checkout.clone();
            let mut components = Path::new(relative).components().peekable();
            while let Some(component) = components.next() {
                from.push(component);
                to.push(component);
                let metadata = fs::symlink_metadata(&from)?;
                if metadata.file_type().is_symlink() {
                    return Err(std::io::Error::other("source symlinks are not supported"));
                }
                if components.peek().is_some() {
                    if !metadata.is_dir() {
                        return Err(std::io::Error::other("source parent is not a directory"));
                    }
                    match fs::create_dir(&to) {
                        Ok(()) => {}
                        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                            let parent = fs::symlink_metadata(&to)?;
                            if !parent.is_dir() || parent.file_type().is_symlink() {
                                return Err(std::io::Error::other(
                                    "destination parent is not a regular directory",
                                ));
                            }
                        }
                        Err(err) => return Err(err),
                    }
                } else {
                    if !metadata.is_file() {
                        return Err(std::io::Error::other("source must be a regular file"));
                    }
                    let mut input = File::open(&from)?;
                    let mut output = OpenOptions::new().write(true).create_new(true).open(&to)?;
                    // Set restrictive source permissions before any secret bytes are written.
                    output.set_permissions(metadata.permissions())?;
                    std::io::copy(&mut input, &mut output)?;
                }
            }
            Ok(())
        })();
        result.map_err(|err| format!("could not copy {relative}: {err}"))?;
    }
    Ok(())
}
