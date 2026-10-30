#![forbid(unsafe_code)]

mod sha256;

#[cfg(windows)]
pub use windows_registry;

use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
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

    /// Manage local AI inference models (Phase 3 WP3)
    Model(ModelArgs),

    /// Manage AI subsystem and hardware diagnostics (Phase 3 WP4)
    Ai(AiArgs),

    /// Manage stored credentials for cloud AI providers (Phase 3 WP7)
    Secret(SecretArgs),
}

/// Arguments for the `ai` subcommand.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AiArgs {
    #[command(subcommand)]
    pub command: AiCommands,
}

/// AI management subcommands (`wavical ai <COMMAND>`).
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum AiCommands {
    /// Inspect system hardware, CPU SIMD features, engine status, and model capabilities
    Doctor(AiDoctorArgs),

    /// Execute low-end hardware performance benchmarks (Phase 3 WP9)
    Bench(AiBenchArgs),
}

/// Arguments for `wavical ai doctor`.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AiDoctorArgs {
    /// Optional project directory to inspect configured models in wavical.config.js
    #[arg(default_value = ".")]
    pub dir: PathBuf,
}

/// Arguments for `wavical ai bench`.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AiBenchArgs {
    /// Project root directory containing wavical.config.js (defaults to current directory)
    #[arg(default_value = ".")]
    pub dir: PathBuf,

    /// Number of measured runs per benchmark metric (default: 5)
    #[arg(long, default_value_t = 5)]
    pub runs: usize,

    /// Number of warmup runs prior to measurement (default: 1)
    #[arg(long, default_value_t = 1)]
    pub warmup: usize,

    /// Optional path to save machine-readable JSON results file
    #[arg(long, short)]
    pub output: Option<PathBuf>,

    /// Run only RAG vector store benchmarks (1k, 10k, 50k chunks)
    #[arg(long)]
    pub rag_only: bool,

    /// Benchmark cloud path if configured
    #[arg(long)]
    pub cloud: bool,

    /// Fixed prompt text for chat benchmarks
    #[arg(long)]
    pub prompt: Option<String>,

    /// Seed for reproducible generation
    #[arg(long, default_value_t = 42)]
    pub seed: u64,
}

/// Arguments for the `secret` subcommand.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct SecretArgs {
    #[command(subcommand)]
    pub command: SecretCommands,
}

/// Secret management subcommands (`wavical secret <COMMAND>`).
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum SecretCommands {
    /// Store an API key in the Windows Credential Store for a provider host
    Set(SecretHostArgs),

    /// Clear a stored API key from the Windows Credential Store for a provider host
    Clear(SecretHostArgs),
}

/// Arguments containing the provider host for secret commands.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct SecretHostArgs {
    /// Provider hostname (e.g. 'api.openai.com')
    pub host: String,
}

/// Arguments for the `model` subcommand.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct ModelArgs {
    #[command(subcommand)]
    pub command: ModelCommands,
}

/// Model management subcommands (`wavical model <COMMAND>`).
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ModelCommands {
    /// List all catalog models and local installation status
    List,

    /// Download and verify a catalog model
    Pull(ModelPullArgs),

    /// Remove an installed model from local storage
    Remove(ModelIdArgs),

    /// Verify SHA-256 integrity and GGUF header of an installed model
    Verify(ModelIdArgs),
}

/// Arguments for `wavical model pull`.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct ModelPullArgs {
    /// Catalog identifier of the model to download (e.g. 'qwen2.5-1.5b-instruct')
    pub id: String,

    /// Explicitly accept the model license non-interactively by exact license ID (e.g. 'Apache-2.0')
    #[arg(long = "accept-license")]
    pub accept_license: Option<String>,
}

/// Arguments for model commands requiring a model ID.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct ModelIdArgs {
    /// Catalog identifier of the model
    pub id: String,
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

    /// AI-assisted prompt to generate application files
    #[arg(long)]
    pub ai: Option<String>,

    /// AI execution routing mode ('local', 'cloud', 'local-first')
    #[arg(long = "ai-mode")]
    pub ai_mode: Option<String>,

    /// Comma-separated list of capabilities to grant (default: none / unselected)
    #[arg(long = "accept-capabilities")]
    pub accept_capabilities: Option<String>,
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

    #[error("model error: {0}")]
    ModelError(#[from] wavical_ai::CommandError),

    #[error("AI scaffolding error: {0}")]
    ScaffoldError(String),

    #[error("AI benchmark error: {0}")]
    BenchmarkError(String),

    #[error("secret error: {0}")]
    SecretError(String),
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

/// Process AI-generated output, validate paths and sizes, scan code for `wavical.invoke(` calls,
/// resolve capability selections (default: none), and safely write the project and `wavical.config.js`.
pub fn process_scaffold_and_write(
    dir: &Path,
    app_id: &str,
    title: &str,
    raw_output: &str,
    accept_capabilities: Option<&str>,
) -> Result<InitSummary, CliError> {
    // 1. Untrusted model output validation: parse JSON / markdown blocks, enforce extensions and size caps
    let files = wavical_ai::parse_scaffold_response(raw_output)
        .map_err(|e| CliError::ScaffoldError(e.to_string()))?;

    // 2. Scan code for used capabilities
    let detected_caps = wavical_ai::scan_used_capabilities(&files);

    // 3. Resolve capability grants (model never decides capabilities, default is none)
    let selected_caps = if let Some(caps_arg) = accept_capabilities {
        let trimmed = caps_arg.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
            Vec::new()
        } else {
            trimmed
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty() && wavical_ipc::KNOWN_CAPABILITIES.contains(&s.as_str()))
                .collect()
        }
    } else if std::io::stdin().is_terminal() && !detected_caps.is_empty() {
        println!("\nDetected capability calls in generated code:");
        for cap in &detected_caps {
            println!("  - {cap}");
        }
        print!(
            "Enter capabilities to grant separated by commas (default: none, press Enter to skip): "
        );
        let _ = std::io::stdout().flush();

        let mut input = String::new();
        let _ = std::io::stdin().read_line(&mut input);
        let trimmed = input.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
            Vec::new()
        } else {
            trimmed
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty() && wavical_ipc::KNOWN_CAPABILITIES.contains(&s.as_str()))
                .collect()
        }
    } else {
        if !detected_caps.is_empty() {
            println!(
                "Detected capability calls in generated code: {:?}",
                detected_caps
            );
            println!(
                "Defaulting to no capabilities granted (run interactively or use --accept-capabilities to grant)."
            );
        }
        Vec::new()
    };

    // 4. Safely write files to <project_dir>/src/ and synthesize wavical.config.js from Rust
    wavical_ai::write_scaffolded_project(dir, app_id, title, &files, &selected_caps)
        .map_err(|e| CliError::ScaffoldError(e.to_string()))?;

    let display_path = if dir.is_relative() && !dir.starts_with(".") {
        format!("./{}", dir.display())
    } else {
        dir.display().to_string()
    };

    Ok(InitSummary {
        app_id: app_id.to_string(),
        title: title.to_string(),
        target_dir: dir.to_path_buf(),
        display_path,
    })
}

/// Scaffold a new Wavical project using AI generation (`wavical init <dir> --ai "<description>"`).
pub fn run_init_ai(
    dir: &Path,
    force: bool,
    description: &str,
    ai_mode_override: Option<&str>,
    accept_capabilities: Option<&str>,
) -> Result<InitSummary, CliError> {
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

    let raw_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("my-app");
    let app_id = sanitize_app_id(raw_name);
    let title = derive_title(&app_id);

    let mode_str = ai_mode_override
        .map(|s| s.to_string())
        .or_else(|| std::env::var("WAVICAL_AI_MODE").ok())
        .unwrap_or_else(|| "local".to_string());

    let mode = wavical_ai::ChatExecutionMode::parse(&mode_str)
        .unwrap_or(wavical_ai::ChatExecutionMode::Local);

    let app_data_path = wavical_ai::wavical_data_dir();
    let cloud_base_url = std::env::var("WAVICAL_CLOUD_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let cloud_model =
        std::env::var("WAVICAL_CLOUD_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
    let cloud_api_key_env =
        std::env::var("WAVICAL_CLOUD_API_KEY_ENV").unwrap_or_else(|_| "OPENAI_API_KEY".to_string());

    let cloud_config = wavical_ai::CloudChatConfig {
        base_url: cloud_base_url,
        model: cloud_model,
        api_key_env: cloud_api_key_env,
        require_consent: true,
    };

    println!("Generating application files with AI for '{app_id}'...");
    let raw_output = wavical_ai::generate_scaffold_code(
        mode,
        &app_data_path,
        &app_id,
        description,
        Some(&cloud_config),
        None,
    )
    .map_err(|e| CliError::ScaffoldError(e.to_string()))?;

    process_scaffold_and_write(dir, &app_id, &title, &raw_output, accept_capabilities)
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
    if let Some(dialog) = &config.dialog
        && let Ok(filters_json) = serde_json::to_string(&dialog.filters)
    {
        cmd.arg("--dialog-filters").arg(filters_json);
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
        Commands::Init(args) => {
            let res = if let Some(ref desc) = args.ai {
                run_init_ai(
                    &args.dir,
                    args.force,
                    desc,
                    args.ai_mode.as_deref(),
                    args.accept_capabilities.as_deref(),
                )
            } else {
                run_init(&args.dir, args.force)
            };

            match res {
                Ok(summary) => {
                    println!("{}", format_init_success(&summary));
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    ExitCode::FAILURE
                }
            }
        }
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
        Commands::Model(args) => match run_model(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::FAILURE
            }
        },
        Commands::Ai(args) => match run_ai(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::FAILURE
            }
        },
        Commands::Secret(args) => match run_secret(&args) {
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

            let open_with_key = CURRENT_USER
                .create(&keys.open_with_progids_key)
                .map_err(|e| {
                    CliError::RegistryError(format!(
                        "Failed to create {}: {e}",
                        keys.open_with_progids_key
                    ))
                })?;
            open_with_key.set_string(&keys.prog_id, "").map_err(|e| {
                CliError::RegistryError(format!(
                    "Failed to register ProgID in {}: {e}",
                    keys.open_with_progids_key
                ))
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
            if let Ok(open_with_key) = CURRENT_USER.create(&keys.open_with_progids_key) {
                let _ = open_with_key.remove_value(&keys.prog_id);
            }
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

/// Execute `wavical model <COMMAND>` subcommands.
pub fn run_model(args: &ModelArgs) -> Result<(), CliError> {
    let models_dir = wavical_ai::wavical_models_dir();
    let data_dir = wavical_ai::wavical_data_dir();

    match args.command {
        ModelCommands::List => {
            let list = wavical_ai::list_models(&models_dir, None);
            println!(
                "{:<24} {:<12} {:<12} {:<12} {:<12}",
                "ID", "KIND", "INSTALLED", "SIZE", "LICENSE"
            );
            println!("{}", "-".repeat(72));
            for info in list {
                let installed_str = if info.installed { "yes" } else { "no" };
                let size_str = wavical_ai::format_size_human(info.size_bytes);
                println!(
                    "{:<24} {:<12} {:<12} {:<12} {:<12}",
                    info.id,
                    info.kind.to_string(),
                    installed_str,
                    size_str,
                    info.license_name
                );
            }
        }
        ModelCommands::Pull(ref pull_args) => {
            let mut stdin = std::io::stdin().lock();
            let mut stdout = std::io::stdout();
            wavical_ai::pull_model(
                &pull_args.id,
                &models_dir,
                &data_dir,
                pull_args.accept_license.as_deref(),
                &wavical_ai::DownloadOptions::default(),
                Some(&mut |current, total| {
                    let percent = if total > 0 {
                        (current as f64 / total as f64) * 100.0
                    } else {
                        0.0
                    };
                    print!(
                        "\rDownloading {}: {:.2} MB / {:.2} MB ({:.1}%)",
                        pull_args.id,
                        current as f64 / (1024.0 * 1024.0),
                        total as f64 / (1024.0 * 1024.0),
                        percent
                    );
                    let _ = std::io::stdout().flush();
                }),
                &mut stdin,
                &mut stdout,
                None,
            )?;
            println!();
        }
        ModelCommands::Remove(ref id_args) => {
            let target = wavical_ai::remove_model(&id_args.id, &models_dir, None)?;
            println!(
                "✓ Successfully removed model '{}' ({})",
                id_args.id,
                target.display()
            );
        }
        ModelCommands::Verify(ref id_args) => {
            println!(
                "Verifying SHA-256 digest and GGUF header for '{}' ...",
                id_args.id
            );
            let target = wavical_ai::verify_model(&id_args.id, &models_dir, None)?;
            println!(
                "✓ Model '{}' is verified and intact ({})",
                id_args.id,
                target.display()
            );
        }
    }

    Ok(())
}

/// Execute AI subsystem commands (`wavical ai <COMMAND>`).
pub fn run_ai(args: &AiArgs) -> Result<(), CliError> {
    match args.command {
        AiCommands::Doctor(ref _doc_args) => {
            let report = wavical_ai::run_ai_doctor(None);
            let output = wavical_ai::format_doctor_report(&report);
            println!("{output}");
            Ok(())
        }
        AiCommands::Bench(ref bench_args) => {
            run_ai_bench(bench_args)?;
            Ok(())
        }
    }
}

/// Execute low-end hardware performance benchmarks (`wavical ai bench`).
pub fn run_ai_bench(args: &AiBenchArgs) -> Result<(), CliError> {
    let cloud_base_url = std::env::var("WAVICAL_CLOUD_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let cloud_model =
        std::env::var("WAVICAL_CLOUD_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
    let cloud_api_key_env =
        std::env::var("WAVICAL_CLOUD_API_KEY_ENV").unwrap_or_else(|_| "OPENAI_API_KEY".to_string());

    let cloud_config = wavical_ai::CloudChatConfig {
        base_url: cloud_base_url,
        model: cloud_model,
        api_key_env: cloud_api_key_env,
        require_consent: false,
    };

    let config = wavical_ai::BenchmarkConfig {
        runs: args.runs,
        warmup_runs: args.warmup,
        prompt: args
            .prompt
            .clone()
            .unwrap_or_else(|| wavical_ai::DEFAULT_BENCHMARK_PROMPT.to_string()),
        seed: args.seed,
    };

    println!(
        "Starting Wavical AI Benchmark (runs: {}, warmup: {}, seed: {})...",
        config.runs, config.warmup_runs, config.seed
    );
    let results =
        wavical_ai::run_ai_benchmarks(&config, args.rag_only, args.cloud, Some(&cloud_config))
            .map_err(CliError::BenchmarkError)?;

    let summary = wavical_ai::format_benchmark_summary(&results);
    println!("{summary}");

    let json_bytes =
        serde_json::to_string_pretty(&results).map_err(|e| CliError::WriteFileFailed {
            path: PathBuf::from("results.json"),
            source: std::io::Error::other(e),
        })?;

    if let Some(ref out_path) = args.output {
        std::fs::write(out_path, &json_bytes).map_err(|e| CliError::WriteFileFailed {
            path: out_path.clone(),
            source: e,
        })?;
        println!(
            "✓ Machine-readable benchmark results saved to {}",
            out_path.display()
        );
    }

    Ok(())
}

/// Execute credential storage commands (`wavical secret <COMMAND>`).
pub fn run_secret(args: &SecretArgs) -> Result<(), CliError> {
    match &args.command {
        SecretCommands::Set(host_args) => {
            let host = wavical_ai::credentials::validate_provider_host(&host_args.host)
                .map_err(|e| CliError::SecretError(e.to_string()))?;

            let secret = rpassword::prompt_password(format!("Enter API key for '{host}': "))
                .map_err(|e| {
                    CliError::SecretError(format!("failed to read hidden password prompt: {e}"))
                })?;

            wavical_ai::credentials::store_secret_in_keyring(host, &secret)
                .map_err(|e| CliError::SecretError(e.to_string()))?;

            println!("✓ Successfully stored API key for '{host}' in Windows Credential Store.");
            Ok(())
        }
        SecretCommands::Clear(host_args) => {
            let host = wavical_ai::credentials::validate_provider_host(&host_args.host)
                .map_err(|e| CliError::SecretError(e.to_string()))?;

            wavical_ai::credentials::clear_secret_from_keyring(host)
                .map_err(|e| CliError::SecretError(e.to_string()))?;

            println!("✓ Cleared API key for '{host}' from Windows Credential Store.");
            Ok(())
        }
    }
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

        // Test model subcommands
        let model_list_cli =
            Cli::try_parse_from(["wavical", "model", "list"]).expect("parse model list");
        match model_list_cli.command {
            Commands::Model(args) => match args.command {
                ModelCommands::List => {}
                _ => panic!("expected model list"),
            },
            _ => panic!("expected model command"),
        }

        let model_pull_cli = Cli::try_parse_from([
            "wavical",
            "model",
            "pull",
            "qwen2.5-1.5b-instruct",
            "--accept-license",
            "Apache-2.0",
        ])
        .expect("parse model pull");
        match model_pull_cli.command {
            Commands::Model(args) => match args.command {
                ModelCommands::Pull(pull_args) => {
                    assert_eq!(pull_args.id, "qwen2.5-1.5b-instruct");
                    assert_eq!(pull_args.accept_license.as_deref(), Some("Apache-2.0"));
                }
                _ => panic!("expected model pull"),
            },
            _ => panic!("expected model command"),
        }

        let model_remove_cli =
            Cli::try_parse_from(["wavical", "model", "remove", "qwen2.5-1.5b-instruct"])
                .expect("parse model remove");
        match model_remove_cli.command {
            Commands::Model(args) => match args.command {
                ModelCommands::Remove(target) => {
                    assert_eq!(target.id, "qwen2.5-1.5b-instruct");
                }
                _ => panic!("expected model remove"),
            },
            _ => panic!("expected model command"),
        }

        let model_verify_cli =
            Cli::try_parse_from(["wavical", "model", "verify", "qwen2.5-1.5b-instruct"])
                .expect("parse model verify");
        match model_verify_cli.command {
            Commands::Model(args) => match args.command {
                ModelCommands::Verify(target) => {
                    assert_eq!(target.id, "qwen2.5-1.5b-instruct");
                }
                _ => panic!("expected model verify"),
            },
            _ => panic!("expected model command"),
        }

        // Blanket --yes or -y flag MUST fail to parse
        assert!(
            Cli::try_parse_from(["wavical", "model", "pull", "qwen2.5-1.5b-instruct", "--yes"])
                .is_err()
        );
        assert!(
            Cli::try_parse_from(["wavical", "model", "pull", "qwen2.5-1.5b-instruct", "-y"])
                .is_err()
        );

        // Test wavical ai doctor
        let ai_doc_cli = Cli::try_parse_from(["wavical", "ai", "doctor"]).expect("parse ai doctor");
        match ai_doc_cli.command {
            Commands::Ai(args) => match args.command {
                AiCommands::Doctor(doc_args) => {
                    assert_eq!(doc_args.dir, PathBuf::from("."));
                }
                _ => panic!("expected doctor command"),
            },
            _ => panic!("expected ai command"),
        }

        // Test wavical ai bench
        let ai_bench_cli = Cli::try_parse_from([
            "wavical",
            "ai",
            "bench",
            "--rag-only",
            "--runs",
            "7",
            "--warmup",
            "2",
            "--output",
            "bench.json",
        ])
        .expect("parse ai bench");
        match ai_bench_cli.command {
            Commands::Ai(args) => match args.command {
                AiCommands::Bench(bench_args) => {
                    assert!(bench_args.rag_only);
                    assert_eq!(bench_args.runs, 7);
                    assert_eq!(bench_args.warmup, 2);
                    assert_eq!(bench_args.output, Some(PathBuf::from("bench.json")));
                }
                _ => panic!("expected bench command"),
            },
            _ => panic!("expected ai command"),
        }

        // Test wavical secret set & clear
        let secret_set_cli = Cli::try_parse_from(["wavical", "secret", "set", "api.openai.com"])
            .expect("parse secret set");
        match secret_set_cli.command {
            Commands::Secret(args) => match args.command {
                SecretCommands::Set(host_args) => {
                    assert_eq!(host_args.host, "api.openai.com");
                }
                _ => panic!("expected secret set"),
            },
            _ => panic!("expected secret command"),
        }

        let secret_clear_cli =
            Cli::try_parse_from(["wavical", "secret", "clear", "api.openai.com"])
                .expect("parse secret clear");
        match secret_clear_cli.command {
            Commands::Secret(args) => match args.command {
                SecretCommands::Clear(host_args) => {
                    assert_eq!(host_args.host, "api.openai.com");
                }
                _ => panic!("expected secret clear"),
            },
            _ => panic!("expected secret command"),
        }

        // Passing key via command-line argument MUST fail to parse (secrets never in arguments)
        assert!(
            Cli::try_parse_from([
                "wavical",
                "secret",
                "set",
                "api.openai.com",
                "--key",
                "secret123"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from(["wavical", "secret", "set", "api.openai.com", "secret123"])
                .is_err()
        );
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
            let open_with_key = CURRENT_USER.open(r"Software\Classes\.wavlife\OpenWithProgids");
            assert!(
                open_with_key.is_ok(),
                "Registry key for .wavlife\\OpenWithProgids should exist in HKCU"
            );
            if let Ok(k) = open_with_key {
                let val: Result<String, _> = k.get_string("Wavical.test-filetypes-lifecycle");
                assert!(
                    val.is_ok(),
                    "ProgID should be registered in OpenWithProgids"
                );
            }
        }

        // 2. Unregister
        run_unregister_filetypes(project_dir).expect("unregister filetypes should succeed");

        #[cfg(windows)]
        {
            use windows_registry::CURRENT_USER;
            let open_with_key = CURRENT_USER.open(r"Software\Classes\.wavlife\OpenWithProgids");
            if let Ok(ref k) = open_with_key {
                let val: Result<String, _> = k.get_string("Wavical.test-filetypes-lifecycle");
                assert!(
                    val.is_err(),
                    "ProgID should be removed from OpenWithProgids on unregister"
                );
            }
            let prog_key = CURRENT_USER.open(r"Software\Classes\Wavical.test-filetypes-lifecycle");
            assert!(
                prog_key.is_err(),
                "Registry key for ProgID must be completely removed"
            );
            // Cleanup test extension key completely so tests leave no trace
            let _ = CURRENT_USER.remove_tree(r"Software\Classes\.wavlife");
        }
    }
}
