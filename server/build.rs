//! Stamps the commit the server is built from into it, so that a session log
//! names the build that wrote it. Same approach as `client/build.rs`, which
//! explains why HEAD and the index are watched and why git must not write the
//! index; outside a git checkout the stamp is `unknown`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty = git(&[
        "--no-optional-locks",
        "status",
        "--porcelain",
        "--untracked-files=no",
    ])
    .is_some_and(|changes| !changes.is_empty());
    let stamp = if dirty {
        format!("{commit}-dirty")
    } else {
        commit
    };
    println!("cargo:rustc-env=OPENROAD_BUILD_COMMIT={stamp}");
    if let Some(dir) = git_dir() {
        for file in ["HEAD", "index"] {
            println!("cargo:rerun-if-changed={}", dir.join(file).display());
        }
    }
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The git directory, following the `gitdir:` file a worktree has instead.
fn git_dir() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent()?;
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let text = std::fs::read_to_string(&dot_git).ok()?;
    Some(root.join(text.strip_prefix("gitdir:")?.trim()))
}
