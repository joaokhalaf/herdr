use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

struct Repo(PathBuf);

impl Repo {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "herdr-git-changes-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let repo = Self(path);
        repo.git(&["init", "--quiet"]);
        repo.git(&["config", "user.name", "Git view test"]);
        repo.git(&["config", "user.email", "git-view@example.invalid"]);
        repo
    }

    fn git(&self, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write(&self, name: &str, text: impl AsRef<[u8]>) {
        std::fs::write(self.0.join(name), text).unwrap();
    }

    fn commit(&self) {
        self.git(&["add", "."]);
        self.git(&["commit", "--quiet", "-m", "fixture"]);
    }

    fn params(&self, result: &GitStatusResult, file: &str) -> GitDiffParams {
        GitDiffParams {
            target: "pane".into(),
            root: result.root.clone(),
            path: file.into(),
            section: result.section,
        }
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn git_changes_separates_staged_local_and_untracked_and_keeps_literal_paths() {
    let repo = Repo::new();
    let unborn = status(&repo.0, GitSection::Local).await.unwrap();
    assert!(unborn.files.is_empty());
    repo.write("file.txt", "original\n");
    repo.commit();
    repo.write("file.txt", "staged\n");
    repo.git(&["add", "."]);
    repo.write("file.txt", "local\n");
    repo.write("-literal[1].txt", "literal\n");
    let local = status(&repo.0, GitSection::Local).await.unwrap();
    assert_eq!(local.files.len(), 2);
    let staged = status(&repo.0, GitSection::Staged).await.unwrap();
    assert_eq!(staged.files.len(), 1);
    assert_eq!(staged.files[0].added, Some(1));
    let local_diff = diff(&repo.0, &repo.params(&local, "file.txt"))
        .await
        .unwrap();
    assert!(local_diff.text.contains("+local"));
    assert!(local_diff.text.contains("-staged"));
    let staged_diff = diff(&repo.0, &repo.params(&staged, "file.txt"))
        .await
        .unwrap();
    assert!(staged_diff.text.contains("+staged"));
    assert!(staged_diff.text.contains("-original"));
    let literal = diff(&repo.0, &repo.params(&local, "-literal[1].txt"))
        .await
        .unwrap();
    assert!(literal.text.contains("+literal"));
    assert!(diff(&repo.0, &repo.params(&local, "../outside"))
        .await
        .is_err());
    let mut stale = repo.params(&local, "file.txt");
    stale.root = "/other-checkout".into();
    assert!(diff(&repo.0, &stale)
        .await
        .unwrap_err()
        .contains("changed checkout"));
}

#[tokio::test]
async fn git_changes_rename_binary_and_bounded_preview() {
    let repo = Repo::new();
    repo.write("old.txt", "original\n");
    repo.commit();
    repo.git(&["mv", "old.txt", "new.txt"]);
    let staged = status(&repo.0, GitSection::Staged).await.unwrap();
    assert_eq!(staged.files[0].old_path.as_deref(), Some("old.txt"));
    let rename = diff(&repo.0, &repo.params(&staged, "new.txt"))
        .await
        .unwrap();
    assert!(rename.text.contains("rename to new.txt"));
    repo.write("binary.dat", b"\0binary");
    repo.write("large.txt", vec![b'x'; DIFF_LIMIT + 1]);
    let local = status(&repo.0, GitSection::Local).await.unwrap();
    assert!(
        diff(&repo.0, &repo.params(&local, "binary.dat"))
            .await
            .unwrap()
            .binary
    );
    let large = diff(&repo.0, &repo.params(&local, "large.txt"))
        .await
        .unwrap();
    assert!(large.truncated);
    assert!(large.text.len() <= DIFF_LIMIT);
    repo.commit();
    repo.write("large.txt", "new line\n".repeat(DIFF_LIMIT / 4));
    let tracked = status(&repo.0, GitSection::Local).await.unwrap();
    let large = diff(&repo.0, &repo.params(&tracked, "large.txt"))
        .await
        .unwrap();
    assert!(large.truncated);
    assert!(large.text.len() <= DIFF_LIMIT);
}

#[test]
fn git_changes_numstat_preserves_rename_paths_and_binary_counts() {
    let counts = parse_numstat(b"12\t3\t\0old\tname\0new\nname\0-\t-\tbinary\0").unwrap();
    assert_eq!(counts["new\nname"], (Some(12), Some(3)));
    assert_eq!(counts["binary"], (None, None));
}

#[tokio::test]
async fn git_changes_linked_checkout_and_detached_head() {
    let repo = Repo::new();
    repo.write("file.txt", "original\n");
    repo.commit();
    let linked = repo.0.join("linked");
    repo.git(&[
        "worktree",
        "add",
        "--quiet",
        "--detach",
        linked.to_str().unwrap(),
    ]);
    std::fs::write(linked.join("file.txt"), "linked edit\n").unwrap();
    let result = status(&linked, GitSection::Local).await.unwrap();
    assert_eq!(
        Path::new(&result.root).canonicalize().unwrap(),
        linked.canonicalize().unwrap()
    );
    assert!(result.branch.contains("HEAD"));
    assert_eq!(result.files.len(), 1);
    assert!(diff(&linked, &repo.params(&result, "file.txt"))
        .await
        .unwrap()
        .text
        .contains("+linked edit"));
}
