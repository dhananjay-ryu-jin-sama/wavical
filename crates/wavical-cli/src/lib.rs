#![forbid(unsafe_code)]

mod sha256;

use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;

/// Wavical command-line interface.
#[derive(Parser, Debug)]
#[command(
    name = "wavical",
    author,
    version,
    about = "Wavical CLI - Developer tool for desktop web applications",
    long_about = None
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// CLI subcommands for Wavical.
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Commands {
    /// Initialize a new Wavical application in <DIR>
    Init(InitArgs),

    /// Start the development server and launch the application shell
    Dev(DevArgs),

    /// Build the application for production
    Build(BuildArgs),

    /// Register file associations declared in wavical.config.js under HKCU\Software\Classes (WP8)
    #[command(name = "register-filetypes")]
    RegisterFiletypes(FiletypesArgs),

    /// Unregister file associations declared in wavical.config.js under HKCU\Software\Classes (WP8)
    #[command(name = "unregister-filetypes")]
    UnregisterFiletypes(FiletypesArgs),
}

/// Arguments for the `register-filetypes` and `unregister-filetypes` subcommands.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct FiletypesArgs {
    /// Project root directory containing wavical.config.js (defaults to current directory)
    #[arg(default_value = ".")]
    pub dir: PathBuf,
}

/// Arguments for the `init` subcommand.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct InitArgs {
    /// Target directory path for the new application
    pub dir: PathBuf,

    /// Overwrite existing files if target directory is not empty
    #[arg(long, short)]
    pub force: bool,
}

/// Arguments for the `dev` subcommand.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct DevArgs {
    /// Project root directory containing wavical.config.js (defaults to current directory)
    #[arg(default_value = ".")]
    pub dir: PathBuf,
}

/// Arguments for the `build` subcommand.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct BuildArgs {
    /// Project root directory containing wavical.config.js (defaults to current directory)
    #[arg(default_value = ".")]
    pub dir: PathBuf,

    /// Output directory for built assets and manifest (defaults to "dist")
    #[arg(long, default_value = "dist")]
    pub outdir: PathBuf,

    /// Build in release mode (reserved for future shell compilation)
    #[arg(long)]
    pub release: bool,
}

/// Errors that can occur during CLI command execution.
#[derive(Debug, Error)]
pub enum CliError {
    #[error("target directory '{0}' already exists and is not empty; use --force to overwrite")]
    DirectoryNotEmpty(PathBuf),

    #[error("failed to create directory '{path}': {source}")]
    CreateDirFailed {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to write file '{path}': {source}")]
    WriteFileFailed {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("wavical.config.js not found in project root '{0}'")]
    ConfigNotFound(PathBuf),

    #[error("invalid config: {0}")]
    ConfigValidation(#[from] wavical_config::ValidationError),

    #[error("failed to locate wavical-shell executable: {0}")]
    ShellNotFound(String),

    #[error("failed to launch shell process: {0}")]
    LaunchShellFailed(#[source] std::io::Error),

    #[error("shell process error: {0}")]
    ShellError(String),

    #[error("source directory '{0}' does not exist")]
    SrcNotFound(PathBuf),

    #[error("failed to copy asset from '{src}' to '{dst}': {source}")]
    CopyAssetFailed {
        src: PathBuf,
        dst: PathBuf,
        source: std::io::Error,
    },

    #[error("Windows registry error: {0}")]
    RegistryError(String),

    #[error("file association error: {0}")]
    FileAssociation(#[from] wavical_ipc::FileAssocValidationError),

    #[error(
        "dangerous output directory '{0}': output directory cannot be project root, parent directory, or inside source directory"
    )]
    DangerousOutDir(PathBuf),
}

/// Summary returned after successful initialization of a new project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitSummary {
    /// Normalized appId used in the generated config
    pub app_id: String,
    /// Human-friendly window/app title
    pub title: String,
    /// Target directory path
    pub target_dir: PathBuf,
    /// Display-friendly path string (e.g. `./my-app`)
    pub display_path: String,
}

/// Sanitize a directory name into a valid alphanumeric/hyphen/underscore appId.
pub fn sanitize_app_id(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('-');
    if trimmed.is_empty() {
        "my-app".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Derive a human-readable title from an appId by splitting words and capitalizing them.
pub fn derive_title(app_id: &str) -> String {
    let words: Vec<&str> = app_id.split(['-', '_']).filter(|s| !s.is_empty()).collect();
    if words.is_empty() {
        return "My App".to_string();
    }
    words
        .iter()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Generate default `wavical.config.js` contents.
pub fn generate_config_js(app_id: &str, title: &str) -> String {
    format!(
        "export default {{\n  appId: '{app_id}',\n  window: {{ width: 1200, height: 800, title: '{title}' }},\n  capabilities: {{}}\n}};\n"
    )
}

/// Generate minimal starter `src/index.html`.
pub fn generate_index_html(title: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n  <meta charset=\"utf-8\">\n  <title>{title}</title>\n  <link rel=\"stylesheet\" href=\"styles.css\">\n</head>\n<body>\n  <h1>Welcome to {title}</h1>\n  <p>Edit src/index.html to get started.</p>\n</body>\n</html>\n"
    )
}

/// Generate basic CSS starter styles for `src/styles.css`.
pub fn generate_styles_css() -> &'static str {
    "/* Basic CSS Reset & Starter Styles */\n*, *::before, *::after {\n  box-sizing: border-box;\n}\n\nbody {\n  margin: 0;\n  padding: 2rem;\n  font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, Helvetica, Arial, sans-serif;\n  background-color: #ffffff;\n  color: #1a1a1a;\n  line-height: 1.5;\n}\n\nh1 {\n  margin-top: 0;\n}\n"
}

/// Generate standard `.gitignore` rules covering Rust, Node, and build artifacts.
pub fn generate_gitignore() -> &'static str {
    "# Build artifacts\ndist/\ntarget/\n\n# Node\nnode_modules/\nnpm-debug.log*\nyarn-debug.log*\nyarn-error.log*\npnpm-debug.log*\n\n# Environment & OS\n.env\n.env.local\n.DS_Store\nThumbs.db\n"
}

/// Scaffold a new Wavical project at the given directory.
pub fn run_init(dir: &Path, force: bool) -> Result<InitSummary, CliError> {
    let is_empty = if dir.exists() {
        let mut entries = std::fs::read_dir(dir).map_err(|e| CliError::CreateDirFailed {
            path: dir.to_path_buf(),
            source: e,
        })?;
        entries.next().is_none()
    } else {
        true
    };

    if !is_empty && !force {
        return Err(CliError::DirectoryNotEmpty(dir.to_path_buf()));
    }

    std::fs::create_dir_all(dir).map_err(|e| CliError::CreateDirFailed {
        path: dir.to_path_buf(),
        source: e,
    })?;

    let src_dir = dir.join("src");
    std::fs::create_dir_all(&src_dir).map_err(|e| CliError::CreateDirFailed {
        path: src_dir.clone(),
        source: e,
    })?;

    let raw_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("my-app");
    let app_id = sanitize_app_id(raw_name);
    let title = derive_title(&app_id);

    // 1. wavical.config.js
    let config_content = generate_config_js(&app_id, &title);
    let config_path = dir.join("wavical.config.js");
    std::fs::write(&config_path, config_content).map_err(|e| CliError::WriteFileFailed {
        path: config_path.clone(),
        source: e,
    })?;

    // 2. src/index.html
    let html_content = generate_index_html(&title);
    let html_path = src_dir.join("index.html");
    std::fs::write(&html_path, html_content).map_err(|e| CliError::WriteFileFailed {
        path: html_path.clone(),
        source: e,
    })?;

    // 3. src/styles.css
    let css_path = src_dir.join("styles.css");
    std::fs::write(&css_path, generate_styles_css()).map_err(|e| CliError::WriteFileFailed {
        path: css_path.clone(),
        source: e,
    })?;

    // 4. .gitignore
    let gitignore_path = dir.join(".gitignore");
    std::fs::write(&gitignore_path, generate_gitignore()).map_err(|e| {
        CliError::WriteFileFailed {
            path: gitignore_path.clone(),
            source: e,
        }
    })?;

    let display_path = if dir.is_relative() && !dir.starts_with(".") {
        format!("./{}", dir.display())
    } else {
        dir.display().to_string()
    };

    Ok(InitSummary {
        app_id,
        title,
        target_dir: dir.to_path_buf(),
        display_path,
    })
}

/// Format the success message printed to the console upon successful `init`.
pub fn format_init_success(summary: &InitSummary) -> String {
    let run_dir = summary
        .target_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(summary.display_path.as_str());
    format!(
        "✓ App created at {}\n  Run: cd {} && wavical dev",
        summary.display_path, run_dir
    )
}

/// Locate the `wavical-shell` executable on the filesystem.
pub fn find_wavical_shell() -> Result<PathBuf, CliError> {
    if let Ok(env_bin) = std::env::var("WAVICAL_SHELL_BIN") {
        let p = PathBuf::from(env_bin);
        if p.is_file() {
            return Ok(p);
        }
    }

    let exe_name = if cfg!(windows) {
        "wavical-shell.exe"
    } else {
        "wavical-shell"
    };

    // 1. Next to current executable or in parent directory (target/debug when run from deps)
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(parent) = current_exe.parent()
    {
        let candidate = parent.join(exe_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
        if let Some(grandparent) = parent.parent() {
            let candidate2 = grandparent.join(exe_name);
            if candidate2.is_file() {
                return Ok(candidate2);
            }
        }
    }

    // 2. Common target directories relative to current directory
    for rel_dir in &[
        "target/debug",
        "../target/debug",
        "../../target/debug",
        "target/release",
    ] {
        let candidate = PathBuf::from(rel_dir).join(exe_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    // 3. Fallback to command name in PATH
    Ok(PathBuf::from(exe_name))
}

/// Launch development server with file watching and live shell reload.
pub fn run_dev(dir: &Path) -> Result<(), CliError> {
    let project_root = if dir == Path::new(".") {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else if dir.is_relative() {
        std::env::current_dir()
            .map(|cwd| cwd.join(dir))
            .unwrap_or_else(|_| dir.to_path_buf())
    } else {
        dir.to_path_buf()
    };

    let config_file = project_root.join("wavical.config.js");
    if !config_file.is_file() {
        return Err(CliError::ConfigNotFound(project_root));
    }

    // 1. Parse and validate config at startup
    let config =
        wavical_config::validate_config(&project_root).map_err(CliError::ConfigValidation)?;

    // 2. Ensure asset directory exists (prefer src/ if index.html exists, fallback to project_root if index.html is there)
    let src_dir = if project_root.join("src").join("index.html").is_file() {
        project_root.join("src")
    } else if project_root.join("index.html").is_file() {
        project_root.clone()
    } else {
        let dir = project_root.join("src");
        if !dir.exists() {
            std::fs::create_dir_all(&dir).map_err(|e| CliError::CreateDirFailed {
                path: dir.clone(),
                source: e,
            })?;
        }
        dir
    };

    // 3. Locate wavical-shell
    let shell_bin = find_wavical_shell()?;

    // 4. Spawn wavical-shell with initial configuration
    let caps_json =
        serde_json::to_string(&config.granted_capabilities()).unwrap_or_else(|_| "[]".to_string());

    let mut cmd = Command::new(&shell_bin);
    cmd.arg("--app-id")
        .arg(&config.app_id)
        .arg("--window-title")
        .arg(config.window_title())
        .arg("--window-width")
        .arg(config.window_width().to_string())
        .arg("--window-height")
        .arg(config.window_height().to_string())
        .arg("--capabilities")
        .arg(&caps_json)
        .arg("--asset-root")
        .arg(&src_dir)
        .arg("--dev")
        .arg("--listen-stdin");

    if !config.persist_window_state() {
        cmd.arg("--persist-state").arg("false");
    }
    if !config.single_instance_or_default() {
        cmd.arg("--single-instance").arg("false");
    }
    if let Some(tray_icon) = config.tray_icon() {
        cmd.arg("--tray-icon").arg(tray_icon);
    }
    if let Some(shortcuts_json) = config
        .shortcuts
        .as_ref()
        .and_then(|s| serde_json::to_string(s).ok())
    {
        cmd.arg("--shortcuts").arg(shortcuts_json);
    }

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(CliError::LaunchShellFailed)?;

    let pid = child.id();
    println!("Dev server running (PID: {pid}). Press Ctrl+C to stop.");
    let _ = std::io::stdout().flush();

    let mut child_stdin = child.stdin.take();

    // 5. Ctrl+C handler
    let running = Arc::new(AtomicBool::new(true));
    let r_ctrlc = running.clone();
    let _ = ctrlc::set_handler(move || {
        r_ctrlc.store(false, Ordering::SeqCst);
    });

    // 6. Setup filesystem watching on src/ and wavical.config.js
    let (watch_tx, watch_rx) = std::sync::mpsc::channel();
    let watcher_res = notify::recommended_watcher(move |res| {
        let _ = watch_tx.send(res);
    });

    let mut _watcher = match watcher_res {
        Ok(mut w) => {
            use notify::Watcher;
            let _ = w.watch(&src_dir, notify::RecursiveMode::Recursive);
            let _ = w.watch(&config_file, notify::RecursiveMode::NonRecursive);
            Some(w)
        }
        Err(err) => {
            eprintln!("warning: failed to start file watcher: {err}");
            None
        }
    };

    let mut last_asset_reload = Instant::now();
    let mut last_config_reload = Instant::now();

    while running.load(Ordering::SeqCst) {
        // Check if child shell has terminated
        match child.try_wait() {
            Ok(Some(status)) => {
                tracing::info!("Shell process exited with status: {status}");
                return Ok(());
            }
            Ok(None) => {}
            Err(e) => {
                return Err(CliError::ShellError(e.to_string()));
            }
        }

        // Wait for file changes with 100ms timeout
        if let Ok(Ok(event)) = watch_rx.recv_timeout(Duration::from_millis(100)) {
            let mut config_changed = false;
            let mut asset_changed = false;

            for path in &event.paths {
                if path.file_name().and_then(|n| n.to_str()) == Some("wavical.config.js") {
                    config_changed = true;
                } else if path.starts_with(&src_dir) {
                    asset_changed = true;
                }
            }

            // Handle config file change
            if config_changed && last_config_reload.elapsed() > Duration::from_millis(150) {
                last_config_reload = Instant::now();
                println!("warning: config changed, reloading...");
                let _ = std::io::stdout().flush();
                match wavical_config::validate_config(&project_root) {
                    Ok(new_config) => {
                        let caps = new_config.granted_capabilities();
                        if let Some(ref mut stdin) = child_stdin {
                            let payload = serde_json::json!({
                                "action": "update_capabilities",
                                "capabilities": caps,
                            });
                            let _ = writeln!(stdin, "{}", payload);
                            let _ = stdin.flush();
                        }
                        println!("✓ Config reloaded. Capability set updated.");
                        let _ = std::io::stdout().flush();
                    }
                    Err(err) => {
                        println!("warning: invalid config: {err}\nkeeping current config running.");
                        let _ = std::io::stdout().flush();
                    }
                }
            }

            // Handle asset file change
            if asset_changed && last_asset_reload.elapsed() > Duration::from_millis(150) {
                let start = Instant::now();
                last_asset_reload = Instant::now();
                println!("info: asset changed, reloading...");
                let _ = std::io::stdout().flush();
                if let Some(ref mut stdin) = child_stdin {
                    let payload = serde_json::json!({ "action": "reload" });
                    let _ = writeln!(stdin, "{}", payload);
                    let _ = stdin.flush();
                }
                println!("✓ Reloaded in {}ms", start.elapsed().as_millis());
                let _ = std::io::stdout().flush();
            }
        }
    }

    // Clean child shutdown
    terminate_child(&mut child, child_stdin.as_mut());
    Ok(())
}

fn terminate_child(child: &mut Child, child_stdin: Option<&mut std::process::ChildStdin>) {
    if let Some(stdin) = child_stdin {
        let _ = writeln!(stdin, "{{\"action\":\"exit\"}}");
        let _ = stdin.flush();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Manifest file (`wavical-manifest.json`) generated in the build output directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WavicalManifest {
    #[serde(rename = "appId")]
    pub app_id: String,
    pub version: String,
    pub capabilities: BTreeMap<String, bool>,
    pub assets: BTreeMap<String, String>,
}

fn has_javascript_files(dir: &Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if has_javascript_files(&p) {
                    return true;
                }
            } else if p.is_file()
                && let Some(ext) = p.extension().and_then(|s| s.to_str())
            {
                let ext_lower = ext.to_ascii_lowercase();
                if matches!(
                    ext_lower.as_str(),
                    "js" | "mjs" | "cjs" | "ts" | "jsx" | "tsx"
                ) {
                    return true;
                }
            }
        }
    }
    false
}

/// Normalize a path by resolving '.' and '..' components without requiring the path to exist on disk.
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => {
                normalized.push(std::path::Component::RootDir.as_os_str())
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(c) => normalized.push(c),
        }
    }
    normalized
}

/// Validate that `out_dir` does not collide with dangerous paths (project root, parent of project, or inside source directory).
pub fn validate_out_dir(
    project_root: &Path,
    src_dir: &Path,
    out_dir: &Path,
) -> Result<(), CliError> {
    let norm_proj = normalize_path(project_root);
    let norm_src = normalize_path(src_dir);
    let norm_out = normalize_path(out_dir);

    if norm_out == norm_proj {
        return Err(CliError::DangerousOutDir(out_dir.to_path_buf()));
    }
    if norm_proj.starts_with(&norm_out) {
        return Err(CliError::DangerousOutDir(out_dir.to_path_buf()));
    }
    if norm_out == norm_src || norm_out.starts_with(&norm_src) {
        return Err(CliError::DangerousOutDir(out_dir.to_path_buf()));
    }

    // Also check canonicalized paths if available for symlink / verbatim prefix resolution
    let can_proj = project_root.canonicalize().ok();
    let can_src = src_dir.canonicalize().ok();
    let can_out = out_dir.canonicalize().ok();

    if let (Some(o), Some(p)) = (&can_out, &can_proj)
        && (o == p || p.starts_with(o))
    {
        return Err(CliError::DangerousOutDir(out_dir.to_path_buf()));
    }
    if let (Some(o), Some(s)) = (&can_out, &can_src)
        && (o == s || o.starts_with(s))
    {
        return Err(CliError::DangerousOutDir(out_dir.to_path_buf()));
    }

    Ok(())
}

/// Remove existing files and subdirectories inside `out_dir` to ensure stale assets are not retained.
pub fn clean_out_dir(out_dir: &Path) -> Result<(), CliError> {
    if !out_dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(out_dir).map_err(|e| CliError::CreateDirFailed {
        path: out_dir.to_path_buf(),
        source: e,
    })? {
        let entry = entry.map_err(|e| CliError::CreateDirFailed {
            path: out_dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path).map_err(|e| CliError::WriteFileFailed {
                path: path.clone(),
                source: e,
            })?;
        } else if path.is_file() {
            std::fs::remove_file(&path).map_err(|e| CliError::WriteFileFailed {
                path: path.clone(),
                source: e,
            })?;
        }
    }
    Ok(())
}

/// Check if a file or directory name matches sensitive, VCS, or OS metadata patterns that should be excluded from builds.
pub fn is_excluded_asset(name: &str) -> bool {
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
    if lower.ends_with(".map") {
        return true;
    }

    false
}

/// Recursively copy assets from `current_src` into `out_root`, computing SHA-256 for each file.
pub fn copy_and_hash_assets(
    src_root: &Path,
    current_src: &Path,
    out_root: &Path,
    assets: &mut BTreeMap<String, String>,
) -> Result<(), CliError> {
    for entry in std::fs::read_dir(current_src).map_err(|e| CliError::CreateDirFailed {
        path: current_src.to_path_buf(),
        source: e,
    })? {
        let entry = entry.map_err(|e| CliError::CreateDirFailed {
            path: current_src.to_path_buf(),
            source: e,
        })?;
        let entry_path = entry.path();
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        if is_excluded_asset(&file_name_str) {
            continue;
        }

        let file_type = entry.file_type().map_err(|e| CliError::CreateDirFailed {
            path: entry_path.clone(),
            source: e,
        })?;

        if file_type.is_dir() {
            let rel_sub = entry_path.strip_prefix(src_root).unwrap_or(&entry_path);
            let dst_sub = out_root.join(rel_sub);
            std::fs::create_dir_all(&dst_sub).map_err(|e| CliError::CreateDirFailed {
                path: dst_sub.clone(),
                source: e,
            })?;
            copy_and_hash_assets(src_root, &entry_path, out_root, assets)?;
        } else if file_type.is_file() {
            let rel_file = entry_path.strip_prefix(src_root).unwrap_or(&entry_path);
            let dst_file = out_root.join(rel_file);
            if let Some(parent) = dst_file.parent() {
                std::fs::create_dir_all(parent).map_err(|e| CliError::CreateDirFailed {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
            std::fs::copy(&entry_path, &dst_file).map_err(|e| CliError::CopyAssetFailed {
                src: entry_path.clone(),
                dst: dst_file.clone(),
                source: e,
            })?;

            let rel_key = rel_file
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");

            let hash = sha256::sha256_file(&dst_file).map_err(|e| CliError::WriteFileFailed {
                path: dst_file.clone(),
                source: e,
            })?;

            assets.insert(rel_key, hash);
        }
    }
    Ok(())
}

/// Execute the `wavical build` command.
pub fn run_build(args: &BuildArgs) -> Result<(), CliError> {
    let project_root = if args.dir == Path::new(".") {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else if args.dir.is_relative() {
        std::env::current_dir()
            .map(|cwd| cwd.join(&args.dir))
            .unwrap_or_else(|_| args.dir.to_path_buf())
    } else {
        args.dir.to_path_buf()
    };

    let config_file = project_root.join("wavical.config.js");
    if !config_file.is_file() {
        return Err(CliError::ConfigNotFound(project_root));
    }

    // 1. Parse and validate config
    let config =
        wavical_config::validate_config(&project_root).map_err(CliError::ConfigValidation)?;

    // 2. Validate src/ directory
    let src_dir = project_root.join("src");
    if !src_dir.is_dir() {
        return Err(CliError::SrcNotFound(src_dir));
    }

    // 3. Warnings
    let index_html = src_dir.join("index.html");
    if !index_html.is_file() {
        eprintln!("warning: 'src/index.html' not found; application may fail to load default page");
    }

    let has_js = has_javascript_files(&src_dir);
    if !has_js {
        eprintln!(
            "warning: no JavaScript files found in src/; application contains only static markup/styles"
        );
    }

    // 4. Resolve outdir and validate it is not dangerous
    let out_dir = if args.outdir.is_absolute() {
        args.outdir.clone()
    } else {
        project_root.join(&args.outdir)
    };

    validate_out_dir(&project_root, &src_dir, &out_dir)?;

    // Clean existing outdir contents before copying to prevent stale file retention (FINDING-C2)
    if out_dir.exists() {
        clean_out_dir(&out_dir)?;
    }

    std::fs::create_dir_all(&out_dir).map_err(|e| CliError::CreateDirFailed {
        path: out_dir.clone(),
        source: e,
    })?;

    // 5. Copy all files from src/ to out_dir and compute hashes
    let mut assets = BTreeMap::new();
    copy_and_hash_assets(&src_dir, &src_dir, &out_dir, &mut assets)?;

    // 6. Generate manifest
    let mut capabilities = BTreeMap::new();
    if let Some(ref caps) = config.capabilities {
        for (k, v) in caps {
            capabilities.insert(k.clone(), *v);
        }
    }

    let manifest = WavicalManifest {
        app_id: config.app_id.clone(),
        version: config.version_or_default().to_string(),
        capabilities,
        assets,
    };

    let manifest_path = out_dir.join("wavical-manifest.json");
    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(|e| CliError::ShellError(e.to_string()))?;
    std::fs::write(&manifest_path, manifest_json).map_err(|e| CliError::WriteFileFailed {
        path: manifest_path.clone(),
        source: e,
    })?;

    if args.release {
        tracing::info!("Build mode: release");
    }

    let out_display = if args.outdir.is_relative() {
        let s = args.outdir.display().to_string();
        if !s.ends_with('/') && !s.ends_with('\\') {
            format!("{s}/")
        } else {
            s
        }
    } else {
        format!("{}/", out_dir.display())
    };

    println!("✓ Build complete. App is in {out_display}. To distribute, zip and ship.");
    Ok(())
}

/// Main entrypoint used by both `wavical` and `wca` binaries.
pub fn main_entry() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init(args) => match run_init(&args.dir, args.force) {
            Ok(summary) => {
                println!("{}", format_init_success(&summary));
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::FAILURE
            }
        },
        Commands::Dev(args) => match run_dev(&args.dir) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::FAILURE
            }
        },
        Commands::Build(args) => match run_build(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::FAILURE
            }
        },
        Commands::RegisterFiletypes(args) => match run_register_filetypes(&args.dir) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::FAILURE
            }
        },
        Commands::UnregisterFiletypes(args) => match run_unregister_filetypes(&args.dir) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::FAILURE
            }
        },
    }
}

/// Register file associations declared in `wavical.config.js` under HKCU\Software\Classes (WP8).
pub fn run_register_filetypes(dir: &Path) -> Result<(), CliError> {
    let project_root = if dir == Path::new(".") {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else if dir.is_relative() {
        std::env::current_dir()
            .map(|cwd| cwd.join(dir))
            .unwrap_or_else(|_| dir.to_path_buf())
    } else {
        dir.to_path_buf()
    };

    let config =
        wavical_config::validate_config(&project_root).map_err(CliError::ConfigValidation)?;

    let exts = match config.file_associations {
        Some(ref list) if !list.is_empty() => list,
        _ => {
            println!("No file associations declared in config.");
            return Ok(());
        }
    };

    let shell_path = find_wavical_shell().unwrap_or_else(|_| {
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("wavical-shell.exe"))
    });
    let exe_str = shell_path.display().to_string();

    #[cfg(windows)]
    {
        use windows_registry::CURRENT_USER;
        for ext in exts {
            let keys = wavical_ipc::build_association_keys(&config.app_id, ext)?;

            let ext_key = CURRENT_USER.create(&keys.ext_key).map_err(|e| {
                CliError::RegistryError(format!("Failed to create {}: {e}", keys.ext_key))
            })?;
            ext_key.set_string("", &keys.prog_id).map_err(|e| {
                CliError::RegistryError(format!("Failed to set {}: {e}", keys.ext_key))
            })?;

            let cmd_key = CURRENT_USER.create(&keys.open_command_key).map_err(|e| {
                CliError::RegistryError(format!("Failed to create {}: {e}", keys.open_command_key))
            })?;
            let command_val = format!(r#""{exe_str}" "%1""#);
            cmd_key.set_string("", &command_val).map_err(|e| {
                CliError::RegistryError(format!("Failed to set command for {}: {e}", keys.prog_id))
            })?;

            println!(
                "✓ Registered file association: .{} -> {}",
                keys.extension, keys.prog_id
            );
        }
    }

    #[cfg(not(windows))]
    {
        let _ = (exts, exe_str);
        println!(
            "File type registration under HKCU\\Software\\Classes is only applicable on Windows."
        );
    }

    Ok(())
}

/// Unregister file associations declared in `wavical.config.js` under HKCU\Software\Classes (WP8).
pub fn run_unregister_filetypes(dir: &Path) -> Result<(), CliError> {
    let project_root = if dir == Path::new(".") {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else if dir.is_relative() {
        std::env::current_dir()
            .map(|cwd| cwd.join(dir))
            .unwrap_or_else(|_| dir.to_path_buf())
    } else {
        dir.to_path_buf()
    };

    let config =
        wavical_config::validate_config(&project_root).map_err(CliError::ConfigValidation)?;

    let exts = match config.file_associations {
        Some(ref list) if !list.is_empty() => list,
        _ => {
            println!("No file associations declared in config.");
            return Ok(());
        }
    };

    #[cfg(windows)]
    {
        use windows_registry::CURRENT_USER;
        for ext in exts {
            let keys = wavical_ipc::build_association_keys(&config.app_id, ext)?;
            let _ = CURRENT_USER.remove_tree(&keys.ext_key);
            let _ = CURRENT_USER.remove_tree(&keys.prog_id_key);
            println!("✓ Unregistered file association: .{}", keys.extension);
        }
    }

    #[cfg(not(windows))]
    {
        let _ = exts;
        println!(
            "File type unregistration under HKCU\\Software\\Classes is only applicable on Windows."
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_app_id() {
        assert_eq!(sanitize_app_id("my-app"), "my-app");
        assert_eq!(sanitize_app_id("My_App-123"), "My_App-123");
        assert_eq!(sanitize_app_id("hello world"), "hello-world");
        assert_eq!(sanitize_app_id("---"), "my-app");
        assert_eq!(sanitize_app_id("special@chars#"), "special-chars");
    }

    #[test]
    fn test_derive_title() {
        assert_eq!(derive_title("my-app"), "My App");
        assert_eq!(derive_title("test-app"), "Test App");
        assert_eq!(derive_title("super_cool_tool"), "Super Cool Tool");
        assert_eq!(derive_title("single"), "Single");
    }

    #[test]
    fn test_format_init_success() {
        let summary = InitSummary {
            app_id: "my-app".to_string(),
            title: "My App".to_string(),
            target_dir: PathBuf::from("my-app"),
            display_path: "./my-app".to_string(),
        };
        let msg = format_init_success(&summary);
        assert!(msg.contains("✓ App created at ./my-app"));
        assert!(msg.contains("Run: cd my-app && wavical dev"));
    }

    #[test]
    fn test_cli_parsing_subcommands() {
        let init_cli = Cli::try_parse_from(["wavical", "init", "my-app"]).expect("parse init");
        match init_cli.command {
            Commands::Init(args) => {
                assert_eq!(args.dir, PathBuf::from("my-app"));
                assert!(!args.force);
            }
            _ => panic!("expected init subcommand"),
        }

        let init_force_cli = Cli::try_parse_from(["wavical", "init", "my-app", "--force"])
            .expect("parse init --force");
        match init_force_cli.command {
            Commands::Init(args) => {
                assert_eq!(args.dir, PathBuf::from("my-app"));
                assert!(args.force);
            }
            _ => panic!("expected init subcommand"),
        }

        let dev_cli = Cli::try_parse_from(["wavical", "dev"]).expect("parse dev default");
        match dev_cli.command {
            Commands::Dev(args) => {
                assert_eq!(args.dir, PathBuf::from("."));
            }
            _ => panic!("expected dev subcommand"),
        }

        let dev_custom_cli =
            Cli::try_parse_from(["wavical", "dev", "my-custom-app"]).expect("parse dev custom dir");
        match dev_custom_cli.command {
            Commands::Dev(args) => {
                assert_eq!(args.dir, PathBuf::from("my-custom-app"));
            }
            _ => panic!("expected dev subcommand"),
        }

        let build_cli = Cli::try_parse_from(["wavical", "build"]).expect("parse build");
        match build_cli.command {
            Commands::Build(args) => {
                assert_eq!(args.dir, PathBuf::from("."));
                assert_eq!(args.outdir, PathBuf::from("dist"));
                assert!(!args.release);
            }
            _ => panic!("expected build subcommand"),
        }

        let build_custom_cli = Cli::try_parse_from([
            "wavical",
            "build",
            "custom-app",
            "--outdir",
            "custom-dist",
            "--release",
        ])
        .expect("parse build custom");
        match build_custom_cli.command {
            Commands::Build(args) => {
                assert_eq!(args.dir, PathBuf::from("custom-app"));
                assert_eq!(args.outdir, PathBuf::from("custom-dist"));
                assert!(args.release);
            }
            _ => panic!("expected build subcommand"),
        }

        let reg_cli = Cli::try_parse_from(["wavical", "register-filetypes", "my-app"])
            .expect("parse register-filetypes");
        match reg_cli.command {
            Commands::RegisterFiletypes(args) => {
                assert_eq!(args.dir, PathBuf::from("my-app"));
            }
            _ => panic!("expected register-filetypes subcommand"),
        }

        let unreg_cli = Cli::try_parse_from(["wavical", "unregister-filetypes", "my-app"])
            .expect("parse unregister-filetypes");
        match unreg_cli.command {
            Commands::UnregisterFiletypes(args) => {
                assert_eq!(args.dir, PathBuf::from("my-app"));
            }
            _ => panic!("expected unregister-filetypes subcommand"),
        }
    }

    #[test]
    fn test_filetype_registration_and_unregistration_lifecycle() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_dir = temp.path();

        let config_js = r#"export default {
            appId: 'test-filetypes-lifecycle',
            fileAssociations: ['wavlife']
        };"#;
        std::fs::write(project_dir.join("wavical.config.js"), config_js).expect("write config");

        // 1. Register
        run_register_filetypes(project_dir).expect("register filetypes should succeed");

        #[cfg(windows)]
        {
            use windows_registry::CURRENT_USER;
            let ext_key = CURRENT_USER.open(r"Software\Classes\.wavlife");
            assert!(
                ext_key.is_ok(),
                "Registry key for .wavlife should exist in HKCU"
            );
            if let Ok(k) = ext_key {
                let val: Result<String, _> = k.get_string("");
                assert_eq!(val.unwrap(), "Wavical.test-filetypes-lifecycle");
            }
        }

        // 2. Unregister
        run_unregister_filetypes(project_dir).expect("unregister filetypes should succeed");

        #[cfg(windows)]
        {
            use windows_registry::CURRENT_USER;
            let ext_key = CURRENT_USER.open(r"Software\Classes\.wavlife");
            assert!(
                ext_key.is_err(),
                "Registry key for .wavlife must be completely removed"
            );
            let prog_key = CURRENT_USER.open(r"Software\Classes\Wavical.test-filetypes-lifecycle");
            assert!(
                prog_key.is_err(),
                "Registry key for ProgID must be completely removed"
            );
        }
    }
}
