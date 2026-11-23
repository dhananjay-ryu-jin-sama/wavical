#![forbid(unsafe_code)]

use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use wavical_shell::test_probe::TEST_PROBE_BINARY_MARKER;
use wavical_shell::{
    WindowConfig, get_asset_root, run_test_probe_with_config, validate_test_proxy_url,
    validate_test_spki_pin,
};

#[derive(Parser, Debug)]
#[command(
    name = "wavical-shell-test-probe",
    about = "Wavical Test-Only Built-App Probe Shell (never shipped in production)",
    version = env!("WAVICAL_VERSION_DISPLAY")
)]
pub struct TestProbeCli {
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

    /// Persist window size, position, and maximized state across sessions
    #[arg(long, env = "WAVICAL_PERSIST_STATE")]
    pub persist_state: Option<bool>,

    /// Enforce single-instance application lock
    #[arg(long, env = "WAVICAL_SINGLE_INSTANCE")]
    pub single_instance: Option<bool>,

    /// Relative path inside src/ to tray icon file
    #[arg(long, env = "WAVICAL_TRAY_ICON")]
    pub tray_icon: Option<String>,

    /// Initial global shortcuts list
    #[arg(long, env = "WAVICAL_SHORTCUTS")]
    pub shortcuts: Option<String>,

    /// Dialog filter mappings
    #[arg(long, env = "WAVICAL_DIALOG_FILTERS")]
    pub dialog_filters: Option<String>,

    /// Optional serialized BackendConfig JSON string
    #[arg(long, env = "WAVICAL_BACKEND_CONFIG")]
    pub backend_config: Option<String>,

    /// Optional project root directory
    #[arg(long, env = "WAVICAL_PROJECT_ROOT")]
    pub project_root: Option<PathBuf>,

    /// Network mode override (ignored for built apps; read strictly from manifest).
    #[arg(long)]
    pub network_mode: Option<String>,

    /// Remote UI URL from window.remoteUrl (built apps use manifest).
    #[arg(long)]
    pub remote_url: Option<String>,

    /// File path where the probe server writes its bound loopback port.
    #[arg(long, env = "WAVICAL_TEST_PROBE_PORT_FILE")]
    pub probe_port_file: Option<PathBuf>,

    /// Loopback HTTP CONNECT proxy URL (`http://127.0.0.1:<port>`) for offline multi-origin tests.
    #[arg(long, env = "WAVICAL_TEST_PROBE_PROXY")]
    pub test_proxy: Option<String>,

    /// Base64 SHA-256 SPKI pin for the offline test proxy certificate.
    #[arg(long, env = "WAVICAL_TEST_PROBE_SPKI")]
    pub test_spki: Option<String>,
}

fn main() {
    let args = TestProbeCli::parse();

    let test_proxy = match args.test_proxy.as_deref() {
        Some(raw) => match validate_test_proxy_url(raw) {
            Ok(p) => Some(p),
            Err(err) => {
                eprintln!("error: invalid --test-proxy: {err}");
                std::process::exit(1);
            }
        },
        None => None,
    };
    let test_spki = match args.test_spki.as_deref() {
        Some(raw) => match validate_test_spki_pin(raw) {
            Ok(s) => Some(s),
            Err(err) => {
                eprintln!("error: invalid --test-spki: {err}");
                std::process::exit(1);
            }
        },
        None => None,
    };

    let asset_root = args.asset_root.unwrap_or_else(get_asset_root);
    let effective_app_id = if args.app_id != "main" || std::env::var_os("WAVICAL_APP_ID").is_some()
    {
        args.app_id.clone()
    } else {
        wavical_shell::net_policy::resolve_manifest_app_id(
            &asset_root,
            args.project_root.as_deref(),
        )
        .unwrap_or_else(|| args.app_id.clone())
    };

    let _ = wavical_shell::init_production_file_logging(&effective_app_id);

    tracing::info!(
        marker = TEST_PROBE_BINARY_MARKER,
        "Starting test-only built-app probe shell"
    );

    if let Err(reason) = wavical_shell::net_policy::verify_safe_mode_startup_integrity(
        &asset_root,
        args.project_root.as_deref(),
        false,
    ) {
        tracing::error!(
            target: "wavical.security",
            security_event = "allowlist_integrity_refused",
            reason = %reason,
            "Refusing to start safe-mode app: application files were changed and cannot start safely"
        );
        std::process::exit(1);
    }

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
    } else {
        granted_caps.push("core.ping".to_string());
        capabilities.grant("core.ping");
    }

    let dyn_caps = Arc::new(RwLock::new(capabilities.clone()));
    let title = args
        .window_title
        .clone()
        .unwrap_or_else(|| effective_app_id.clone());

    let backend_config = args
        .backend_config
        .as_deref()
        .and_then(|raw| serde_json::from_str::<wavical_config::BackendConfig>(raw).ok());

    let network_policy = wavical_shell::net_policy::resolve_shell_network_policy_with_remote(
        &asset_root,
        args.project_root.as_deref(),
        false,
        args.network_mode.as_deref(),
        None,
        args.remote_url.as_deref(),
        granted_caps,
    );

    let config = WindowConfig {
        window_id: effective_app_id,
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
        listen_stdin: false,
        persist_state: args.persist_state.unwrap_or(false),
        single_instance: args.single_instance.unwrap_or(false),
        tray_icon: args.tray_icon,
        shortcuts: Vec::new(),
        dialog_filters: HashMap::new(),
        backend_config,
        network_policy,
        project_root: args.project_root,
        backend_service: None,
        asset_root: None,
        remote_debugging_port: None,
        test_proxy,
        test_spki,
        ..Default::default()
    };

    run_test_probe_with_config(asset_root, config, args.probe_port_file);
}
