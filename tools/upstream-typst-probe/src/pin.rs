//! The upstream release every fixture is generated from.

use std::{fs, path::Path, process::Command};

use serde::Deserialize;

use crate::Result;

/// `avenger-typst-label/tests/fixtures/typst-pin.toml`.
#[derive(Debug, Deserialize)]
pub struct Pin {
    pub version: String,
    pub commit: String,
}

impl Pin {
    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = repo_root.join("avenger-typst-label/tests/fixtures/typst-pin.toml");
        let text = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        Ok(toml::from_str(&text)?)
    }

    /// Fails unless the `../typst` checkout this probe links is the pinned commit with no local
    /// changes to its crates or lockfile.
    pub fn verify_checkout(&self, typst_dir: &Path) -> Result<()> {
        let head = git(typst_dir, &["rev-parse", "HEAD"])?;
        if head != self.commit {
            return Err(format!(
                "../typst is at {head}, but fixtures are pinned to Typst {} ({}); run `git -C ../typst checkout v{}`",
                self.version, self.commit, self.version
            )
            .into());
        }
        let dirty = git(
            typst_dir,
            &[
                "status",
                "--porcelain",
                "--untracked-files=no",
                "--",
                "crates",
                "Cargo.lock",
            ],
        )?;
        if !dirty.is_empty() {
            return Err(format!("../typst has local changes:\n{dirty}").into());
        }
        Ok(())
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output()?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed in {}: {}",
            args.join(" "),
            dir.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}
