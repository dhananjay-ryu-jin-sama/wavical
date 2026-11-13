#![forbid(unsafe_code)]

//! Backend build, dependency installation, exclusion filtering, and runtime bundling for `wavical build`.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use wavical_config::BackendConfig;
use wavical_protocol::{
    APPROVED_RUNTIMES, RuntimeArch, find_approved_runtime, resolve_runtime_version,
};

use crate::CliError;
use crate::runtime::{self, RuntimeMetadata};
use crate::sha256;

/// Documented built-in exclusion rules for `wavical build` (web assets and backend).
pub const DOCUMENTED_BUILD_EXCLUDES: &[&str] = &[
    ".env*",
    ".git*",
    ".svn",
    ".hg",
    "test/",
    "tests/",
    "__tests__/",
    "spec/",
    "__mocks__/",
    "coverage/",
    "*.map (unless --include-source-maps)",
    ".DS_Store",
    "Thumbs.db",
    "desktop.ini",
];

const MAX_COPY_DEPTH: usize = 64;

/// Backend section of `wavical-manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackendBuildManifest {
    pub entry: String,
    pub files: BTreeMap<String, String>,
    pub install_scripts_allowed: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub skipped_install_scripts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub install_scripts_note: Option<String>,
}

/// Runtime section of `wavical-manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeBuildManifest {
    pub mode: String,
    pub runtime: String,
    pub version: String,
    pub arch: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub executable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub license_sha256: Option<String>,
}

/// Result of building the backend and optionally bundling the runtime.
pub struct BackendBuildOutput {
    pub backend: BackendBuildManifest,
    pub runtime: RuntimeBuildManifest,
}

/// Load optional `.wavicalignore` patterns from the project root.
pub fn load_wavicalignore(project_root: &Path) -> Result<Vec<String>, CliError> {
    let ignore_path = project_root.join(".wavicalignore");
    if !ignore_path.is_file() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&ignore_path).map_err(|e| CliError::WriteFileFailed {
        path: ignore_path,
        source: e,
    })?;

    let mut patterns = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        patterns.push(trimmed.to_string());
    }
    Ok(patterns)
}

/// Format the active documented exclusion list for display.
pub fn format_documented_ignore_list(
    include_source_maps: bool,
    extra_patterns: &[String],
) -> String {
    let mut items: Vec<String> = DOCUMENTED_BUILD_EXCLUDES
        .iter()
        .filter(|&&p| !(include_source_maps && p.starts_with("*.map")))
        .map(|s| (*s).to_string())
        .collect();
    for pat in extra_patterns {
        items.push(pat.clone());
    }
    items.join(", ")
}

/// Match a single ignore pattern against an entry name or forward-slash relative path.
fn matches_ignore_pattern(pattern: &str, name: &str, rel_path: &str, is_dir: bool) -> bool {
    let pat = pattern.trim().replace('\\', "/");
    if pat.is_empty() {
        return false;
    }

    let dir_only = pat.ends_with('/');
    let clean_pat = pat.trim_end_matches('/').trim_start_matches('/');
    if clean_pat.is_empty() {
        return false;
    }
    if dir_only && !is_dir {
        return false;
    }

    let pat_lower = clean_pat.to_ascii_lowercase();
    let name_lower = name.to_ascii_lowercase();
    let rel_lower = rel_path.to_ascii_lowercase();

    if let Some(suffix) = pat_lower.strip_prefix('*') {
        return name_lower.ends_with(suffix) || rel_lower.ends_with(suffix);
    }
    if let Some(prefix) = pat_lower.strip_suffix('*') {
        return name_lower.starts_with(prefix) || rel_lower.starts_with(prefix);
    }

    name_lower == pat_lower || rel_lower == pat_lower
}

/// Determine whether a file or directory should be excluded from the build output.
pub fn is_excluded_build_entry(
    name: &str,
    rel_path: &str,
    is_dir: bool,
    include_source_maps: bool,
    extra_patterns: &[String],
) -> bool {
    let lower = name.to_ascii_lowercase();

    if lower.starts_with(".env") {
        return true;
    }
    if lower.starts_with(".git") || lower == ".svn" || lower == ".hg" {
        return true;
    }
    if lower == ".ds_store" || lower == "thumbs.db" || lower == "desktop.ini" {
        return true;
    }
    if !include_source_maps && lower.ends_with(".map") {
        return true;
    }
    if is_dir
        && matches!(
            lower.as_str(),
            "test" | "tests" | "__tests__" | "spec" | "__mocks__" | "coverage"
        )
    {
        return true;
    }

    for pat in extra_patterns {
        if matches_ignore_pattern(pat, name, rel_path, is_dir) {
            return true;
        }
    }

    false
}

/// Resolve the backend directory from project root and `BackendConfig`.
pub fn resolve_backend_dir(
    project_root: &Path,
    backend_cfg: &BackendConfig,
) -> Result<(PathBuf, String), CliError> {
    let entry_rel = backend_cfg
        .entry
        .clone()
        .unwrap_or_else(|| "backend/main.js".to_string());

    let default_backend_dir = project_root.join("backend");
    if default_backend_dir.is_dir() {
        let stripped = entry_rel
            .replace('\\', "/")
            .strip_prefix("backend/")
            .map(|s| s.to_string())
            .unwrap_or_else(|| entry_rel.clone());
        return Ok((default_backend_dir, stripped));
    }

    let entry_path = project_root.join(&entry_rel);
    if let Some(parent) = entry_path.parent()
        && parent != project_root
        && parent.is_dir()
    {
        let file_name = entry_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("main.js")
            .to_string();
        return Ok((parent.to_path_buf(), file_name));
    }

    Err(CliError::BackendBuildFailed(format!(
        "backend directory not found in '{}'",
        project_root.display()
    )))
}

/// Verify that the managed runtime exists in cache and passes its SHA-256 hash check.
pub fn resolve_and_verify_managed_runtime(
    backend_cfg: &BackendConfig,
) -> Result<
    (
        PathBuf,
        RuntimeMetadata,
        &'static wavical_protocol::RuntimeManifestEntry,
    ),
    CliError,
> {
    let pinned_version = backend_cfg.version.as_deref().ok_or_else(|| {
        CliError::BackendBuildFailed("backend.version must be set when backend is enabled".into())
    })?;

    let available: Vec<&str> = APPROVED_RUNTIMES
        .iter()
        .filter(|e| e.runtime == backend_cfg.runtime)
        .map(|e| e.version)
        .collect();

    let resolved_version =
        resolve_runtime_version(pinned_version, backend_cfg.allow_patch_updates, &available)
            .map_err(|e| CliError::BackendBuildFailed(e.to_string()))?;

    let arch = RuntimeArch::current();
    let entry = find_approved_runtime(&backend_cfg.runtime, &resolved_version, arch)
        .map_err(|e| CliError::BackendBuildFailed(e.to_string()))?;

    let runtime_dir = runtime::cache_dir_for_runtime(entry);
    if !runtime_dir.is_dir() {
        return Err(CliError::BackendBuildFailed(format!(
            "managed runtime '{} v{}' ({}) is not installed in cache '{}'; run 'wavical runtime install {}' first",
            entry.runtime,
            entry.version,
            entry.arch.as_str(),
            runtime_dir.display(),
            entry.version
        )));
    }

    let meta_path = runtime_dir.join(".wavical-runtime.json");
    if !meta_path.is_file() {
        return Err(CliError::BackendBuildFailed(format!(
            "runtime metadata missing at '{}'; reinstall with 'wavical runtime install {}'",
            meta_path.display(),
            entry.version
        )));
    }

    let meta_str = fs::read_to_string(&meta_path).map_err(|e| CliError::WriteFileFailed {
        path: meta_path.clone(),
        source: e,
    })?;
    let meta: RuntimeMetadata = serde_json::from_str(&meta_str).map_err(|e| {
        CliError::BackendBuildFailed(format!(
            "invalid runtime metadata '{}': {e}",
            meta_path.display()
        ))
    })?;

    let exe_path = runtime_dir.join(&meta.executable_name);
    runtime::verify_runtime_executable_hash(&exe_path, &meta.executable_sha256)
        .map_err(|e| CliError::BackendBuildFailed(e.to_string()))?;

    Ok((runtime_dir, meta, entry))
}

/// Inspect `package-lock.json` / `npm-shrinkwrap.json` and installed `node_modules`
/// for production packages that declare lifecycle install scripts.
fn collect_packages_with_install_scripts(backend_dir: &Path) -> Vec<String> {
    let mut packages = BTreeSet::new();

    // 1. Check lockfile `packages` map for `hasInstallScript: true` on non-dev packages
    let lock_path = if backend_dir.join("package-lock.json").is_file() {
        backend_dir.join("package-lock.json")
    } else {
        backend_dir.join("npm-shrinkwrap.json")
    };

    if let Ok(lock_str) = fs::read_to_string(&lock_path)
        && let Ok(lock_json) = serde_json::from_str::<serde_json::Value>(&lock_str)
        && let Some(pkgs_obj) = lock_json.get("packages").and_then(|v| v.as_object())
    {
        for (pkg_key, pkg_val) in pkgs_obj {
            if pkg_key.is_empty() {
                continue;
            }
            let is_dev = pkg_val
                .get("dev")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let has_script = pkg_val
                .get("hasInstallScript")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !is_dev && has_script {
                let name = pkg_key
                    .rsplit("node_modules/")
                    .next()
                    .unwrap_or(pkg_key)
                    .to_string();
                if !name.is_empty() {
                    packages.insert(name);
                }
            }
        }
    }

    // 2. Scan installed node_modules/**/package.json for preinstall/install/postinstall
    let nm_dir = backend_dir.join("node_modules");
    if nm_dir.is_dir() {
        scan_node_modules_for_scripts(&nm_dir, &mut packages, 0);
    }

    packages.into_iter().collect()
}

fn scan_node_modules_for_scripts(dir: &Path, packages: &mut BTreeSet<String>, depth: usize) {
    if depth > MAX_COPY_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if name.starts_with('@') {
                scan_node_modules_for_scripts(&path, packages, depth + 1);
            } else {
                let pkg_json = path.join("package.json");
                if pkg_json.is_file()
                    && let Ok(raw) = fs::read_to_string(&pkg_json)
                    && let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw)
                    && let Some(scripts) = val.get("scripts").and_then(|s| s.as_object())
                {
                    let has_install = scripts.contains_key("preinstall")
                        || scripts.contains_key("install")
                        || scripts.contains_key("postinstall");
                    if has_install {
                        let pkg_name = val
                            .get("name")
                            .and_then(|n| n.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or(name);
                        packages.insert(pkg_name);
                    }
                }
                let nested_nm = path.join("node_modules");
                if nested_nm.is_dir() {
                    scan_node_modules_for_scripts(&nested_nm, packages, depth + 1);
                }
            }
        }
    }
}

/// Run `npm ci` (never `npm install`) in `backend_dir` using the verified managed runtime.
fn run_npm_ci_with_managed_runtime(
    backend_dir: &Path,
    runtime_dir: &Path,
    exe_path: &Path,
    allow_install_scripts: bool,
) -> Result<(), CliError> {
    let mut npm_args = vec!["ci", "--omit=dev"];
    if !allow_install_scripts {
        npm_args.push("--ignore-scripts");
    }

    let npm_cli_js = runtime_dir
        .join("node_modules")
        .join("npm")
        .join("bin")
        .join("npm-cli.js");

    let mut cmd = if npm_cli_js.is_file() {
        let mut c = Command::new(exe_path);
        c.arg(&npm_cli_js);
        c
    } else if cfg!(windows) && runtime_dir.join("npm.cmd").is_file() {
        Command::new(runtime_dir.join("npm.cmd"))
    } else if !cfg!(windows) && runtime_dir.join("npm").is_file() {
        Command::new(runtime_dir.join("npm"))
    } else {
        Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" })
    };

    cmd.args(&npm_args).current_dir(backend_dir);

    // Prepend managed runtime dir to PATH so any child node invocations resolve to managed node
    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    let sep = if cfg!(windows) { ";" } else { ":" };
    let new_path = format!(
        "{}{}{}",
        runtime_dir.display(),
        sep,
        existing_path.to_string_lossy()
    );
    cmd.env("PATH", new_path);

    let output = cmd.output().map_err(|e| {
        CliError::BackendBuildFailed(format!(
            "failed to run 'npm ci' with managed runtime '{}': {e}",
            exe_path.display()
        ))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(CliError::BackendBuildFailed(format!(
            "npm ci failed (exit {}):\n{}\n{}",
            output.status,
            stderr.trim(),
            stdout.trim()
        )));
    }

    Ok(())
}

/// Recursively copy backend files into `out_backend_dir` while applying exclusion rules
/// and computing SHA-256 digests for every copied file.
pub fn copy_and_hash_backend_files(
    backend_root: &Path,
    current_src: &Path,
    out_backend_dir: &Path,
    include_source_maps: bool,
    extra_ignore: &[String],
    files_manifest: &mut BTreeMap<String, String>,
    depth: usize,
) -> Result<(), CliError> {
    if depth > MAX_COPY_DEPTH {
        return Err(CliError::BackendBuildFailed(
            "maximum directory depth exceeded while copying backend files".into(),
        ));
    }

    let entries = fs::read_dir(current_src).map_err(|e| CliError::CreateDirFailed {
        path: current_src.to_path_buf(),
        source: e,
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| CliError::CreateDirFailed {
            path: current_src.to_path_buf(),
            source: e,
        })?;
        let entry_path = entry.path();
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        let rel_path = entry_path.strip_prefix(backend_root).unwrap_or(&entry_path);
        let rel_key = rel_path
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");

        let is_dir = entry_path.is_dir();

        if is_excluded_build_entry(
            &file_name_str,
            &rel_key,
            is_dir,
            include_source_maps,
            extra_ignore,
        ) {
            continue;
        }

        if is_dir {
            let dst_sub = out_backend_dir.join(rel_path);
            fs::create_dir_all(&dst_sub).map_err(|e| CliError::CreateDirFailed {
                path: dst_sub.clone(),
                source: e,
            })?;
            copy_and_hash_backend_files(
                backend_root,
                &entry_path,
                out_backend_dir,
                include_source_maps,
                extra_ignore,
                files_manifest,
                depth + 1,
            )?;
        } else if entry_path.is_file() {
            let dst_file = out_backend_dir.join(rel_path);
            if let Some(parent) = dst_file.parent() {
                fs::create_dir_all(parent).map_err(|e| CliError::CreateDirFailed {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
            fs::copy(&entry_path, &dst_file).map_err(|e| CliError::CopyAssetFailed {
                src: entry_path.clone(),
                dst: dst_file.clone(),
                source: e,
            })?;

            let hash = sha256::sha256_file(&dst_file).map_err(|e| CliError::WriteFileFailed {
                path: dst_file.clone(),
                source: e,
            })?;
            files_manifest.insert(rel_key, hash);
        }
    }

    Ok(())
}

/// Locate the license file in the runtime directory.
fn find_runtime_license_file(runtime_dir: &Path) -> Option<PathBuf> {
    for candidate in ["LICENSE", "LICENSE.md", "LICENSE.txt", "license"] {
        let p = runtime_dir.join(candidate);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Build the backend and optionally bundle the verified runtime into `out_dir`.
pub fn build_backend_and_runtime(
    project_root: &Path,
    out_dir: &Path,
    backend_cfg: &BackendConfig,
    allow_install_scripts: bool,
    bundle_runtime: bool,
    include_source_maps: bool,
    extra_ignore: &[String],
) -> Result<BackendBuildOutput, CliError> {
    // 1. Resolve backend directory and require lockfile
    let (backend_dir, entry_rel) = resolve_backend_dir(project_root, backend_cfg)?;
    let has_lockfile = backend_dir.join("package-lock.json").is_file()
        || backend_dir.join("npm-shrinkwrap.json").is_file();
    if !has_lockfile {
        return Err(CliError::MissingBackendLockfile(backend_dir));
    }

    // 2. Resolve and hash-verify the managed runtime BEFORE using or bundling it
    let (runtime_dir, meta, entry) = resolve_and_verify_managed_runtime(backend_cfg)?;
    let src_exe = runtime_dir.join(&meta.executable_name);

    // 3. Run `npm ci` with the managed runtime (never `npm install`)
    run_npm_ci_with_managed_runtime(&backend_dir, &runtime_dir, &src_exe, allow_install_scripts)?;

    // 4. Identify packages with install scripts
    let packages_with_scripts = collect_packages_with_install_scripts(&backend_dir);
    let (skipped_install_scripts, install_scripts_note) = if allow_install_scripts {
        eprintln!(
            "warning: --allow-install-scripts enabled; npm lifecycle install scripts were executed during build"
        );
        (
            Vec::new(),
            Some(
                "WARNING: --allow-install-scripts was enabled; package install scripts were executed during build."
                    .to_string(),
            ),
        )
    } else {
        if packages_with_scripts.is_empty() {
            println!(
                "Backend dependencies installed via npm ci --ignore-scripts (no install scripts present)."
            );
        } else {
            println!(
                "Skipped install scripts for {} package(s): {}",
                packages_with_scripts.len(),
                packages_with_scripts.join(", ")
            );
        }
        (packages_with_scripts, None)
    };

    // 5. Copy backend code and production dependencies into out_dir/backend/ and hash every file
    let out_backend_dir = out_dir.join("backend");
    fs::create_dir_all(&out_backend_dir).map_err(|e| CliError::CreateDirFailed {
        path: out_backend_dir.clone(),
        source: e,
    })?;

    let mut backend_files = BTreeMap::new();
    copy_and_hash_backend_files(
        &backend_dir,
        &backend_dir,
        &out_backend_dir,
        include_source_maps,
        extra_ignore,
        &mut backend_files,
        0,
    )?;

    let backend_manifest = BackendBuildManifest {
        entry: format!("backend/{entry_rel}"),
        files: backend_files,
        install_scripts_allowed: allow_install_scripts,
        skipped_install_scripts,
        install_scripts_note,
    };

    // 6. Bundle runtime if requested, otherwise record shared-cache mode
    let runtime_manifest = if bundle_runtime {
        let license_src = find_runtime_license_file(&runtime_dir)
            .ok_or_else(|| CliError::RuntimeLicenseMissing(runtime_dir.clone()))?;

        let out_runtime_dir = out_dir.join("runtime");
        fs::create_dir_all(&out_runtime_dir).map_err(|e| CliError::CreateDirFailed {
            path: out_runtime_dir.clone(),
            source: e,
        })?;

        let dst_exe = out_runtime_dir.join(&meta.executable_name);
        fs::copy(&src_exe, &dst_exe).map_err(|e| CliError::CopyAssetFailed {
            src: src_exe.clone(),
            dst: dst_exe.clone(),
            source: e,
        })?;

        // Re-verify copied runtime binary hash
        runtime::verify_runtime_executable_hash(&dst_exe, &meta.executable_sha256)
            .map_err(|e| CliError::BackendBuildFailed(e.to_string()))?;

        let dst_license = out_runtime_dir.join("LICENSE");
        fs::copy(&license_src, &dst_license).map_err(|e| CliError::CopyAssetFailed {
            src: license_src,
            dst: dst_license.clone(),
            source: e,
        })?;
        let license_hash =
            sha256::sha256_file(&dst_license).map_err(|e| CliError::WriteFileFailed {
                path: dst_license,
                source: e,
            })?;

        RuntimeBuildManifest {
            mode: "bundled".to_string(),
            runtime: entry.runtime.to_string(),
            version: entry.version.to_string(),
            arch: entry.arch.as_str().to_string(),
            executable: Some(format!("runtime/{}", meta.executable_name)),
            sha256: Some(format!("sha256:{}", meta.executable_sha256)),
            license: Some("runtime/LICENSE".to_string()),
            license_sha256: Some(license_hash),
        }
    } else {
        RuntimeBuildManifest {
            mode: "shared-cache".to_string(),
            runtime: entry.runtime.to_string(),
            version: entry.version.to_string(),
            arch: entry.arch.as_str().to_string(),
            executable: None,
            sha256: None,
            license: None,
            license_sha256: None,
        }
    };

    Ok(BackendBuildOutput {
        backend: backend_manifest,
        runtime: runtime_manifest,
    })
}
