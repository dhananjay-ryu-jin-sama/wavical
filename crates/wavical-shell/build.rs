#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=WAVICAL_GIT_COMMIT");
    println!("cargo:rerun-if-env-changed=WAVICAL_GIT_DIRTY");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    emit_rerun_hints(&manifest_dir);

    let env_commit = std::env::var("WAVICAL_GIT_COMMIT")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let (commit, git_available) = match env_commit {
        Some(c) => (c, true),
        None => match run_git(&manifest_dir, &["rev-parse", "--short", "HEAD"]) {
            Some(c) if !c.is_empty() => (c, true),
            _ => ("unknown".to_string(), false),
        },
    };

    let dirty = match std::env::var("WAVICAL_GIT_DIRTY") {
        Ok(val) => matches!(
            val.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        ),
        Err(_) => {
            if git_available {
                run_git(&manifest_dir, &["status", "--porcelain"])
                    .map(|out| !out.trim().is_empty())
                    .unwrap_or(false)
            } else {
                false
            }
        }
    };

    let pkg_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".to_string());
    let dirty_suffix = if dirty { "-dirty" } else { "" };
    let display_version = format!("{pkg_version} ({commit}{dirty_suffix})");

    println!("cargo:rustc-env=WAVICAL_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=WAVICAL_GIT_DIRTY={dirty}");
    println!("cargo:rustc-env=WAVICAL_VERSION_DISPLAY={display_version}");
}

fn emit_rerun_hints(manifest_dir: &Path) {
    let src_dir = manifest_dir.join("src");
    if src_dir.exists() {
        println!("cargo:rerun-if-changed={}", src_dir.display());
    }

    let Some(git_dir_str) = run_git(manifest_dir, &["rev-parse", "--git-dir"]) else {
        return;
    };
    let git_dir_raw = PathBuf::from(git_dir_str.trim());
    let git_dir = if git_dir_raw.is_absolute() {
        git_dir_raw
    } else {
        manifest_dir.join(git_dir_raw)
    };

    let head_path = git_dir.join("HEAD");
    if head_path.is_file() {
        println!("cargo:rerun-if-changed={}", head_path.display());
        if let Ok(head_contents) = std::fs::read_to_string(&head_path)
            && let Some(ref_rel) = head_contents.trim().strip_prefix("ref: ")
        {
            let ref_path = git_dir.join(ref_rel.trim());
            if ref_path.is_file() {
                println!("cargo:rerun-if-changed={}", ref_path.display());
            }
        }
    }

    let index_path = git_dir.join("index");
    if index_path.is_file() {
        println!("cargo:rerun-if-changed={}", index_path.display());
    }
}

fn run_git(cwd: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout)
        .ok()
        .map(|s| s.trim_end().to_string())
}
