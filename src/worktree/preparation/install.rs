use std::fs;
use std::path::Path;

/// Runs in the creation worker against the selected base's new checkout.
/// Only known executable names are returned; project metadata is never shell code.
pub(super) fn command(checkout: &Path) -> Result<Vec<String>, String> {
    if !regular_file(checkout, "package.json")? {
        return Err("automatic dependency installation needs package.json in the checkout root; configure command for other projects".into());
    }
    let content = fs::read(checkout.join("package.json"))
        .map_err(|_| "could not read package.json for automatic dependency installation")?;
    let package: serde_json::Value = serde_json::from_slice(&content)
        .map_err(|_| "invalid package.json for automatic dependency installation")?;
    if !package.is_object() {
        return Err("package.json must contain an object".into());
    }
    let manager = if let Some(declared) = package.get("packageManager") {
        declared
            .as_str()
            .and_then(|value| value.split('@').next())
            .filter(|name| matches!(*name, "npm" | "pnpm" | "bun" | "yarn"))
            .ok_or(
                "packageManager must name npm, pnpm, bun or yarn; configure command to override",
            )?
    } else {
        let mut detected = None;
        for (manager, files) in [
            ("pnpm", &["pnpm-lock.yaml"][..]),
            ("npm", &["package-lock.json", "npm-shrinkwrap.json"][..]),
            ("bun", &["bun.lock", "bun.lockb"][..]),
            ("yarn", &["yarn.lock"][..]),
        ] {
            let mut found = false;
            for file in files {
                found |= regular_file(checkout, file)?;
            }
            if found {
                if detected.is_some() {
                    return Err("multiple package managers have lockfiles; set packageManager in package.json or configure command".into());
                }
                detected = Some(manager);
            }
        }
        detected.ok_or("could not detect a package manager; set packageManager in package.json or configure command")?
    };
    Ok(vec![manager.into(), "install".into()])
}

fn regular_file(checkout: &Path, name: &str) -> Result<bool, String> {
    match fs::symlink_metadata(checkout.join(name)) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(format!(
            "{name} must be a regular file for package manager detection"
        )),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(format!(
            "could not inspect {name} for package manager detection"
        )),
    }
}
