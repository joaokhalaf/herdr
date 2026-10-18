//! Bounded, read-only Git queries. Called on demand by the server, never by rendering.
use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path};
use std::process::Stdio;

use tokio::io::AsyncReadExt;

use crate::api::schema::{
    GitDiffParams, GitDiffResult, GitFileChange, GitSection, GitStatusResult,
};

const STATUS_LIMIT: usize = 2 * 1024 * 1024;
const DIFF_LIMIT: usize = 256 * 1024;

async fn command(cwd: &Path, args: &[&str], limit: usize) -> Result<(Vec<u8>, bool), String> {
    let mut command = crate::noninteractive_process::command("git");
    command
        .args([
            "--no-optional-locks",
            "--literal-pathspecs",
            "-c",
            "core.fsmonitor=false",
            "-C",
        ])
        .arg(cwd)
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut command = tokio::process::Command::from(command);
    command.kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let stdout = child.stdout.take().ok_or("Git stdout unavailable")?;
    let stderr = child.stderr.take().ok_or("Git stderr unavailable")?;
    let operation = async {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let read_stdout = async {
            stdout
                .take(limit as u64 + 1)
                .read_to_end(&mut output)
                .await?;
            let truncated = output.len() > limit;
            if truncated {
                let _ = child.kill().await;
            }
            let status = child.wait().await?;
            std::io::Result::Ok((status, truncated))
        };
        let read_stderr = async { stderr.take(16 * 1024).read_to_end(&mut errors).await };
        let ((status, truncated), _) =
            tokio::try_join!(read_stdout, read_stderr).map_err(|error| error.to_string())?;
        output.truncate(limit);
        if !status.success() && !truncated {
            return Err(String::from_utf8_lossy(&errors).trim().to_owned());
        }
        Ok((output, truncated))
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), operation)
        .await
        .map_err(|_| "Git command timed out".to_owned())?
}

fn utf8(bytes: &[u8]) -> Result<String, String> {
    String::from_utf8(bytes.to_vec())
        .map_err(|_| "Git returned a path that is not valid UTF-8".into())
}

pub(crate) async fn status(cwd: &Path, section: GitSection) -> Result<GitStatusResult, String> {
    let (root, _) = command(cwd, &["rev-parse", "--show-toplevel"], STATUS_LIMIT).await?;
    let root = utf8(&root)?.trim_end_matches('\n').to_owned();
    let root_path = Path::new(&root);
    let (bytes, truncated) = command(
        root_path,
        &[
            "status",
            "--porcelain=v1",
            "--branch",
            "-z",
            "--untracked-files=all",
        ],
        STATUS_LIMIT,
    )
    .await?;
    if truncated {
        return Err("Too many changed files to display (2 MiB limit)".into());
    }
    let mut records = bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    let header = records.next().ok_or("Git status header missing")?;
    let branch = utf8(header)?
        .trim_start_matches("## ")
        .split("...")
        .next()
        .unwrap_or("HEAD")
        .trim_start_matches("No commits yet on ")
        .trim_start_matches("Initial commit on ")
        .to_owned();
    let mut files = Vec::new();
    for_record(&mut records, section, &mut files)?;
    let mut args = vec!["diff", "--no-ext-diff", "--no-textconv", "--numstat", "-z"];
    if section == GitSection::Staged {
        args.push("--cached");
    }
    let (counts, truncated) = command(root_path, &args, STATUS_LIMIT).await?;
    if truncated {
        return Err("Git change statistics exceed the 2 MiB limit".into());
    }
    let counts = parse_numstat(&counts)?;
    for file in &mut files {
        if let Some((added, removed)) = counts.get(&file.path) {
            file.added = *added;
            file.removed = *removed;
        }
    }
    Ok(GitStatusResult {
        terminal_id: None,
        root,
        branch,
        section,
        files,
    })
}

fn for_record<'a>(
    records: &mut impl Iterator<Item = &'a [u8]>,
    section: GitSection,
    files: &mut Vec<GitFileChange>,
) -> Result<(), String> {
    while let Some(record) = records.next() {
        if record.len() < 4 || record[2] != b' ' {
            return Err("Invalid Git status record".into());
        }
        let old_path = if record[..2].iter().any(|byte| matches!(byte, b'R' | b'C')) {
            Some(utf8(records.next().ok_or("Missing rename source")?)?)
        } else {
            None
        };
        let code = record[if section == GitSection::Staged { 0 } else { 1 }];
        if code == b' ' || (code == b'?' && section == GitSection::Staged) {
            continue;
        }
        files.push(GitFileChange {
            path: utf8(&record[3..])?,
            old_path,
            status: char::from(code).to_string(),
            added: None,
            removed: None,
        });
    }
    Ok(())
}

type LineCounts = HashMap<String, (Option<u64>, Option<u64>)>;

fn parse_numstat(bytes: &[u8]) -> Result<LineCounts, String> {
    let mut records = bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    let mut counts = HashMap::new();
    while let Some(record) = records.next() {
        let mut fields = record.splitn(3, |byte| *byte == b'\t');
        let added = fields.next().ok_or("Missing additions")?;
        let removed = fields.next().ok_or("Missing deletions")?;
        let name = fields.next().ok_or("Missing numstat path")?;
        let name = if name.is_empty() {
            let _old = records.next().ok_or("Missing numstat rename source")?;
            records.next().ok_or("Missing numstat rename destination")?
        } else {
            name
        };
        let number = |value: &[u8]| {
            std::str::from_utf8(value)
                .ok()
                .and_then(|value| value.parse().ok())
        };
        counts.insert(utf8(name)?, (number(added), number(removed)));
    }
    Ok(counts)
}

pub(crate) async fn diff(cwd: &Path, params: &GitDiffParams) -> Result<GitDiffResult, String> {
    let status = status(cwd, params.section).await?;
    if status.root != params.root {
        return Err("The terminal changed checkout; refresh the Git view".into());
    }
    let file = status
        .files
        .iter()
        .find(|file| file.path == params.path)
        .ok_or("The file no longer has changes in this section; refresh the Git view")?;
    if Path::new(&file.path)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Invalid repository-relative path".into());
    }
    if file.status == "?" {
        let root = status.root;
        let name = file.path.clone();
        return tokio::task::spawn_blocking(move || untracked(Path::new(&root), &name))
            .await
            .map_err(|error| error.to_string())?;
    }
    let mut args = vec![
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--unified=3",
    ];
    if params.section == GitSection::Staged {
        args.push("--cached");
    }
    args.push("--");
    if let Some(old) = &file.old_path {
        args.push(old);
    }
    args.push(&file.path);
    let (bytes, truncated) = command(Path::new(&status.root), &args, DIFF_LIMIT).await?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let binary = text.lines().any(|line| line.starts_with("Binary files "));
    Ok(GitDiffResult {
        text,
        truncated,
        binary,
    })
}

fn untracked(root: &Path, name: &str) -> Result<GitDiffResult, String> {
    let path = root.join(name);
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let parent = path
        .parent()
        .ok_or("Missing file parent")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !parent.starts_with(root) {
        return Err("The path is outside the checkout".into());
    }
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    let bytes = if metadata.file_type().is_symlink() {
        std::fs::read_link(&path)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .as_bytes()
            .to_vec()
    } else if metadata.is_file() {
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .map_err(|error| error.to_string())?
            .take(DIFF_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        bytes
    } else {
        return Err("This entry is not a regular file".into());
    };
    let binary = bytes.contains(&0);
    let mut truncated = bytes.len() > DIFF_LIMIT;
    let text = if binary {
        "Binary file — no text preview".into()
    } else {
        let content = String::from_utf8_lossy(&bytes[..bytes.len().min(DIFF_LIMIT)]);
        let mut text = format!("--- /dev/null\n+++ b/{name}\n");
        for line in content.lines() {
            if text.len() + line.len() + 2 > DIFF_LIMIT {
                truncated = true;
                break;
            }
            text.push('+');
            text.push_str(line);
            text.push('\n');
        }
        text
    };
    Ok(GitDiffResult {
        text,
        truncated,
        binary,
    })
}

#[cfg(test)]
mod tests;
