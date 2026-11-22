#![forbid(unsafe_code)]

use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use wavical_shell::{WindowConfig, get_asset_root, run_with_config};

#[derive(Parser, Debug)]
#[command(
    name = "wavical-shell",
    about = "Wavical Phase 0/1 Native Application Shell",
    version = env!("WAVICAL_VERSION_DISPLAY")
)]
pub struct ShellCli {
    /// Unique application identifier
    #[arg(long, env = "WAVICAL_APP_ID", default_value = "main")]
    pub app_id: String,

    /// Application window title
    #[arg(long, env = "WAVICAL_WINDOW_TITLE")]
    pub window_title: Option<String>,

    /// Initial window width in pixels
    #[arg(long, env = "WAVICAL_WINDOW_WIDTH", default_value = "1200")]
    pub window_width: u32,

    /// Initial window height in pixels
    #[arg(long, env = "WAVICAL_WINDOW_HEIGHT", default_value = "800")]
    pub window_height: u32,

    /// Capabilities grant list (JSON array/object or comma-separated list)
    #[arg(long, env = "WAVICAL_CAPABILITIES")]
    pub capabilities: Option<String>,

    /// Root directory containing web application assets
    #[arg(long, env = "WAVICAL_ASSET_ROOT")]
    pub asset_root: Option<PathBuf>,

    /// Listen on stdin for dynamic dev commands (reload, update_capabilities, exit)
    #[cfg(debug_assertions)]
    #[arg(long)]
    pub listen_stdin: bool,

    /// Persist window size, position, and maximized state across sessions
    #[arg(long, env = "WAVICAL_PERSIST_STATE")]
    pub persist_state: Option<bool>,

    /// Enforce single-instance application lock
    #[arg(long, env = "WAVICAL_SINGLE_INSTANCE")]
    pub single_instance: Option<bool>,

    /// Relative path inside src/ to tray icon file (WP7)
    #[arg(long, env = "WAVICAL_TRAY_ICON")]
    pub tray_icon: Option<String>,

    /// Initial global shortcuts list (JSON array or comma-separated list) (WP8)
    #[arg(long, env = "WAVICAL_SHORTCUTS")]
    pub shortcuts: Option<String>,

    /// Dialog filter mappings (JSON object) (WP4)
    #[arg(long, env = "WAVICAL_DIALOG_FILTERS")]
    pub dialog_filters: Option<String>,

    /// Optional serialized BackendConfig JSON string (Phase 4 WP5)
    #[arg(long, env = "WAVICAL_BACKEND_CONFIG")]
    pub backend_config: Option<String>,

    /// Optional project root directory (Phase 4 WP5)
    #[arg(long, env = "WAVICAL_PROJECT_ROOT")]
    pub project_root: Option<PathBuf>,

    /// Run in development server mode
    #[arg(long)]
    pub dev: bool,

    /// Network mode for dev commands (open, safe, ultra-safe). Ignored for built apps.
    #[arg(long)]
    pub network_mode: Option<String>,

    /// Dev server URL (localhost/127.0.0.1 with port) for dev commands only.
    #[arg(long)]
    pub dev_url: Option<String>,

    /// Remote UI URL from window.remoteUrl (dev commands only; built apps use manifest).
    #[arg(long)]
    pub remote_url: Option<String>,

    /// Record blocked events to .wavical/recorded-blocks.json (dev-safe / dev-ultra-safe only).
    #[arg(long)]
    pub record: bool,
}

fn main() {
    tracing_subscriber::fmt::init();
    let args = ShellCli::parse();

    let asset_root = args.asset_root.unwrap_or_else(get_asset_root);

    let mut capabilities = wavical_ipc::Capabilities::new();
    let mut granted_caps = Vec::new();
    if let Some(ref caps_raw) = args.capabilities {
        let trimmed = caps_raw.trim();
        if trimmed.starts_with('[') {
            if let Ok(list) = serde_json::from_str::<Vec<String>>(trimmed) {
                for cap in list {
                    granted_caps.push(cap.clone());
                    capabilities.grant(cap);
                }
            }
        } else if trimmed.starts_with('{') {
            if let Ok(map) = serde_json::from_str::<HashMap<String, bool>>(trimmed) {
                for (cap, granted) in map {
                    if granted {
                        granted_caps.push(cap.clone());
                        capabilities.grant(cap);
                    }
                }
            }
        } else {
            for cap in trimmed.split(',') {
                let c = cap.trim();
                if !c.is_empty() {
                    granted_caps.push(c.to_string());
                    capabilities.grant(c);
                }
            }
        }
    } else if !args.dev {
        // Phase 0 backward compatibility: grant core.ping for bare execution
        granted_caps.push("core.ping".to_string());
        capabilities.grant("core.ping");
    }

    #[cfg(debug_assertions)]
    let listen_stdin = args.listen_stdin || args.dev;
    #[cfg(not(debug_assertions))]
    let listen_stdin = false;
    let dyn_caps = Arc::new(RwLock::new(capabilities.clone()));
    let title = args
        .window_title
        .clone()
        .unwrap_or_else(|| args.app_id.clone());

    let initial_shortcuts = if let Some(ref raw) = args.shortcuts {
        let trimmed = raw.trim();
        if trimmed.starts_with('[') {
            serde_json::from_str::<Vec<String>>(trimmed).unwrap_or_default()
        } else {
            trimmed
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
    } else {
        Vec::new()
    };

    let initial_dialog_filters = if let Some(ref raw) = args.dialog_filters {
        if let Ok(map) = serde_json::from_str::<HashMap<String, (String, Vec<String>)>>(raw) {
            map
        } else if let Ok(map) = serde_json::from_str::<HashMap<String, serde_json::Value>>(raw) {
            let mut res = HashMap::new();
            for (k, v) in map {
                if let (Some(name), Some(exts)) = (
                    v.get("name").and_then(|n| n.as_str()),
                    v.get("extensions").and_then(|e| e.as_array()),
                ) {
                    let exts_vec: Vec<String> = exts
                        .iter()
                        .filter_map(|e| e.as_str().map(|s| s.to_string()))
                        .collect();
                    res.insert(k, (name.to_string(), exts_vec));
                }
            }
            res
        } else {
            HashMap::new()
        }
    } else {
        HashMap::new()
    };

    let backend_config = args
        .backend_config
        .as_deref()
        .and_then(|raw| serde_json::from_str::<wavical_config::BackendConfig>(raw).ok());

    let mut network_policy = wavical_shell::net_policy::resolve_shell_network_policy_with_remote(
        &asset_root,
        args.project_root.as_deref(),
        args.dev,
        args.network_mode.as_deref(),
        args.dev_url.as_deref(),
        args.remote_url.as_deref(),
        granted_caps,
    );
    if args.dev && args.record {
        let rec_path = args
            .project_root
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(".wavical")
            .join("recorded-blocks.json");
        network_policy.recorder = Some(std::sync::Arc::new(
            wavical_shell::net_policy::BlockRecorder::new(Some(rec_path)),
        ));
    }

    let config = WindowConfig {
        window_id: args.app_id,
        allowed_origins: vec![
            "http://wavical.localhost".to_string(),
            "wavical://localhost".to_string(),
            "https://wavical.localhost".to_string(),
        ],
        capabilities,
        custom_csp: None,
        dynamic_capabilities: Some(dyn_caps),
        initial_width: Some(args.window_width),
        initial_height: Some(args.window_height),
        title: Some(title),
        listen_stdin,
        persist_state: args.persist_state.unwrap_or(true),
        single_instance: args.single_instance.unwrap_or(true),
        tray_icon: args.tray_icon,
        shortcuts: initial_shortcuts,
        dialog_filters: initial_dialog_filters,
        backend_config,
        network_policy,
        project_root: args.project_root,
        backend_service: None,
        asset_root: None,
        ..Default::default()
    };

    run_with_config(asset_root, config);
}
