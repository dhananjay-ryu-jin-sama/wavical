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
    version
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

    /// Run in development server mode
    #[arg(long)]
    pub dev: bool,
}

fn main() {
    tracing_subscriber::fmt::init();
    let args = ShellCli::parse();

    let asset_root = args.asset_root.unwrap_or_else(get_asset_root);

    let mut capabilities = wavical_ipc::Capabilities::new();
    if let Some(ref caps_raw) = args.capabilities {
        let trimmed = caps_raw.trim();
        if trimmed.starts_with('[') {
            if let Ok(list) = serde_json::from_str::<Vec<String>>(trimmed) {
                for cap in list {
                    capabilities.grant(cap);
                }
            }
        } else if trimmed.starts_with('{') {
            if let Ok(map) = serde_json::from_str::<HashMap<String, bool>>(trimmed) {
                for (cap, granted) in map {
                    if granted {
                        capabilities.grant(cap);
                    }
                }
            }
        } else {
            for cap in trimmed.split(',') {
                let c = cap.trim();
                if !c.is_empty() {
                    capabilities.grant(c);
                }
            }
        }
    } else if !args.dev {
        // Phase 0 backward compatibility: grant core.ping for bare execution
        capabilities.grant("core.ping");
    }

    let listen_stdin = args.listen_stdin || args.dev;
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
    };

    run_with_config(asset_root, config);
}
