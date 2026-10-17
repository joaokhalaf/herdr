use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    source: PathBuf,
    checkout: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "herdr-preparation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let source = root.join("source");
        let checkout = root.join("checkout");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&checkout).unwrap();
        Self {
            root,
            source,
            checkout,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn copies_only_selected_files_and_keeps_nested_paths() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.source.join("config")).unwrap();
    fs::write(fixture.source.join("config/local.env"), "test fixture").unwrap();
    fs::write(fixture.source.join("not-selected"), "not copied").unwrap();
    copy_files(
        &fixture.source,
        &fixture.checkout,
        &["config/local.env".into()],
    )
    .unwrap();
    assert_eq!(
        fs::read(fixture.checkout.join("config/local.env")).unwrap(),
        b"test fixture"
    );
    assert!(!fixture.checkout.join("not-selected").exists());
}

#[test]
fn automatic_install_uses_each_supported_lockfile_in_the_new_checkout() {
    for (lockfile, manager) in [
        ("pnpm-lock.yaml", "pnpm"),
        ("package-lock.json", "npm"),
        ("npm-shrinkwrap.json", "npm"),
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
        ("yarn.lock", "yarn"),
    ] {
        let fixture = Fixture::new();
        fs::write(
            fixture.source.join("package.json"),
            r#"{"packageManager":"pnpm@10.0.0"}"#,
        )
        .unwrap();
        fs::write(fixture.checkout.join("package.json"), "{}").unwrap();
        fs::write(fixture.checkout.join(lockfile), "fixture").unwrap();
        let rule = WorktreePreparationConfig {
            install_dependencies: true,
            ..Default::default()
        };
        assert_eq!(
            command(&fixture.checkout, &rule).unwrap(),
            [manager, "install"],
            "{lockfile}"
        );
    }
}

#[test]
fn explicit_package_manager_wins_over_lockfiles_and_never_supplies_shell_code() {
    let fixture = Fixture::new();
    fs::write(fixture.checkout.join("pnpm-lock.yaml"), "fixture").unwrap();
    fs::write(fixture.checkout.join("package-lock.json"), "fixture").unwrap();
    let rule = WorktreePreparationConfig {
        install_dependencies: true,
        ..Default::default()
    };
    for manager in ["pnpm", "npm", "yarn", "bun"] {
        fs::write(
            fixture.checkout.join("package.json"),
            format!(r#"{{"packageManager":"{manager}@1.0.0+sha224.fixture"}}"#),
        )
        .unwrap();
        assert_eq!(
            command(&fixture.checkout, &rule).unwrap(),
            [manager, "install"]
        );
    }
    for content in [
        "invalid json",
        "[]",
        r#"{"packageManager":null}"#,
        r#"{"packageManager":23}"#,
        r#"{"packageManager":"unknown@1.0.0"}"#,
        r#"{"packageManager":"npm; touch marker"}"#,
    ] {
        fs::write(fixture.checkout.join("package.json"), content).unwrap();
        assert!(command(&fixture.checkout, &rule).is_err());
    }
    assert!(!fixture.checkout.join("marker").exists());
}

#[test]
fn automatic_install_is_opt_in_and_manual_command_overrides_detection() {
    let fixture = Fixture::new();
    let mut rule = WorktreePreparationConfig::default();
    assert!(command(&fixture.checkout, &rule).unwrap().is_empty());
    rule.install_dependencies = true;
    assert!(command(&fixture.checkout, &rule).is_err());
    fs::write(fixture.checkout.join("package.json"), "{}").unwrap();
    assert!(command(&fixture.checkout, &rule).is_err());
    fs::write(fixture.checkout.join("yarn.lock"), "fixture").unwrap();
    fs::write(fixture.checkout.join("bun.lock"), "fixture").unwrap();
    assert!(command(&fixture.checkout, &rule)
        .unwrap_err()
        .contains("multiple"));
    rule.command = vec!["custom-installer".into(), "--project".into()];
    assert_eq!(command(&fixture.checkout, &rule).unwrap(), rule.command);
    // Two lockfile formats belonging to the same manager are not ambiguous.
    fs::remove_file(fixture.checkout.join("yarn.lock")).unwrap();
    fs::write(fixture.checkout.join("bun.lockb"), "fixture").unwrap();
    rule.command.clear();
    assert_eq!(
        command(&fixture.checkout, &rule).unwrap(),
        ["bun", "install"]
    );
}

#[cfg(unix)]
#[test]
fn automatic_install_rejects_symlinked_metadata() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    fs::write(fixture.source.join("package.json"), "{}").unwrap();
    let rule = WorktreePreparationConfig {
        install_dependencies: true,
        ..Default::default()
    };
    symlink(
        fixture.source.join("package.json"),
        fixture.checkout.join("package.json"),
    )
    .unwrap();
    assert!(command(&fixture.checkout, &rule).is_err());
    fs::remove_file(fixture.checkout.join("package.json")).unwrap();
    fs::write(fixture.checkout.join("package.json"), "{}").unwrap();
    symlink(
        fixture.source.join("package.json"),
        fixture.checkout.join("bun.lock"),
    )
    .unwrap();
    assert!(command(&fixture.checkout, &rule).is_err());
}

#[test]
fn never_overwrites_existing_files_or_copies_directories() {
    let fixture = Fixture::new();
    fs::write(fixture.source.join(".env"), "source fixture").unwrap();
    fs::write(fixture.checkout.join(".env"), "checkout fixture").unwrap();
    assert!(copy_files(&fixture.source, &fixture.checkout, &[".env".into()]).is_err());
    assert_eq!(
        fs::read(fixture.checkout.join(".env")).unwrap(),
        b"checkout fixture"
    );
    fs::create_dir(fixture.source.join("directory")).unwrap();
    assert!(copy_files(&fixture.source, &fixture.checkout, &["directory".into()]).is_err());
    assert!(copy_files(&fixture.source, &fixture.checkout, &["missing".into()]).is_err());
}

#[test]
fn validates_paths_and_matches_only_the_configured_repository() {
    let fixture = Fixture::new();
    for invalid in [
        "",
        "../outside",
        "/outside",
        ".git/config",
        "nested/.GIT/config",
        "./file",
    ] {
        assert!(validate_relative_file(invalid).is_err(), "{invalid}");
    }
    let mut rule = WorktreePreparationConfig {
        repository: fixture.source.display().to_string(),
        install_dependencies: false,
        copy_files: vec![".env".into()],
        command: vec!["pnpm".into(), "install".into()],
    };
    assert!(for_repository(&fixture.checkout, &[rule.clone()])
        .unwrap()
        .copy_files
        .is_empty());
    assert_eq!(
        for_repository(&fixture.source, &[rule.clone()])
            .unwrap()
            .command,
        rule.command
    );
    assert!(for_repository(&fixture.source, &[rule.clone(), rule.clone()]).is_err());
    rule.command = vec!["sh".into(), "bad\nargument".into()];
    assert!(for_repository(&fixture.source, &[rule]).is_err());
}

#[cfg(unix)]
#[test]
fn preserves_private_permissions_and_rejects_source_or_destination_symlinks() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let fixture = Fixture::new();
    fs::write(fixture.source.join(".env"), "private fixture").unwrap();
    fs::set_permissions(
        fixture.source.join(".env"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    copy_files(&fixture.source, &fixture.checkout, &[".env".into()]).unwrap();
    assert_eq!(
        fs::metadata(fixture.checkout.join(".env"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    symlink(
        fixture.source.join(".env"),
        fixture.source.join("linked.env"),
    )
    .unwrap();
    assert!(copy_files(&fixture.source, &fixture.checkout, &["linked.env".into()]).is_err());
    fs::create_dir(fixture.source.join("config")).unwrap();
    fs::write(fixture.source.join("config/file"), "fixture").unwrap();
    symlink(&fixture.source, fixture.checkout.join("config")).unwrap();
    assert!(copy_files(&fixture.source, &fixture.checkout, &["config/file".into()]).is_err());
    assert!(!fixture.source.join("file").exists());
}

#[cfg(unix)]
#[test]
fn setup_wrapper_preserves_arguments_cwd_and_reports_failure_before_shell() {
    let fixture = Fixture::new();
    let command = vec![
        "sh".into(),
        "-c".into(),
        "printf '%s' \"$1\" > marker; exit 7".into(),
        "fixture".into(),
        "literal $(not-executed) ' spaced".into(),
    ];
    let argv = crate::platform::worktree_preparation_argv(&command);
    let output = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(&fixture.checkout)
        .env("HERDR_WORKTREE_SHELL", "/bin/true")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(fixture.checkout.join("marker")).unwrap(),
        command[4]
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("setup exited with status 7"));
}
