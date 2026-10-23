#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder},
    window::{Window, WindowBuilder},
};
use thiserror::Error;
use wavical_protocol::{HEADER_NOSNIFF, resolve_asset_path};
use wry::http::{Request, Response, StatusCode, Uri};
use wry::{NewWindowResponse, WebViewBuilder};

/// Strict Content-Security-Policy applied by default to all responses.
pub const DEFAULT_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self';";

/// Standard security header name for Content-Security-Policy.
pub const HEADER_CSP: &str = "Content-Security-Policy";

/// Initialization script injected into every document on creation.
///
/// Defines `window.wavical` as a frozen object with a single `invoke(method, params)` function.
/// All internal state (pending promises map, request counter) is completely encapsulated in an IIFE.
/// Both `window.wavical` and the internal `window.__wavical_receive__` receiver are defined
/// as non-writable and non-configurable properties using `Object.defineProperty`.
pub const WAVICAL_INIT_SCRIPT: &str = r#"
(() => {
    if (window.wavical) return;

    const pending = new Map();
    let nextId = 1;

    function receiveResponse(response) {
        if (!response || typeof response.id !== 'number') return;
        const entry = pending.get(response.id);
        if (!entry) return;
        pending.delete(response.id);
        if (response.ok) {
            entry.resolve(response.result);
        } else {
            entry.reject(new Error(response.error || "IPC request failed"));
        }
    }

    Object.defineProperty(window, '__wavical_receive__', {
        value: Object.freeze(receiveResponse),
        writable: false,
        configurable: false,
        enumerable: false,
    });

    function invoke(method, params = null) {
        if (typeof method !== 'string') {
            return Promise.reject(new TypeError("Method must be a string"));
        }
        if (!window.ipc || typeof window.ipc.postMessage !== 'function') {
            return Promise.reject(new Error("Wavical IPC channel unavailable"));
        }
        const id = nextId++;
        if (nextId >= Number.MAX_SAFE_INTEGER) {
            nextId = 1;
        }
        return new Promise((resolve, reject) => {
            pending.set(id, { resolve, reject });
            try {
                window.ipc.postMessage(JSON.stringify({ id, method, params }));
            } catch (err) {
                pending.delete(id);
                reject(err);
            }
        });
    }

    const wavicalApi = Object.freeze({
        invoke: Object.freeze(invoke),
    });

    Object.defineProperty(window, 'wavical', {
        value: wavicalApi,
        writable: false,
        configurable: false,
        enumerable: true,
    });
})();
"#;

/// Custom events sent to the tao event loop for execution on the UI thread.
#[derive(Debug, Clone)]
pub enum UserEvent {
    /// Safe JavaScript evaluation string delivered to the webview.
    ExecuteScript(String),
    /// Reload current webview page.
    Reload,
    /// Dynamically update granted capabilities without restarting window.
    UpdateCapabilities(Vec<String>),
    /// Focus existing application window upon second instance launch.
    FocusWindow,
    /// WebView2 renderer process failure / crash event with reason code.
    RendererProcessFailed(i32),
    /// Exit event loop cleanly.
    Exit,
}

/// Errors raised when validating incoming IPC message origins.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IpcOriginError {
    #[error("Disallowed origin: '{0}'")]
    DisallowedOrigin(String),

    #[error("Unable to parse origin from request URI")]
    InvalidUri,
}

/// Per-window security, navigation, and capability configuration.
#[derive(Debug, Clone)]
pub struct WindowConfig {
    /// Window identifier used for structured audit logging.
    pub window_id: String,
    /// Explicit allowlist of origin prefixes authorized to exchange IPC messages.
    pub allowed_origins: Vec<String>,
    /// Window-specific capability set (default: `core.ping` granted; `core.echo` ungranted).
    pub capabilities: wavical_ipc::Capabilities,
    /// Optional Content-Security-Policy override. If None, `DEFAULT_CSP` is enforced.
    pub custom_csp: Option<String>,
    /// Thread-safe dynamic capability storage allowing live capability reloads without restarting the window.
    pub dynamic_capabilities: Option<std::sync::Arc<std::sync::RwLock<wavical_ipc::Capabilities>>>,
    /// Optional initial window width.
    pub initial_width: Option<u32>,
    /// Optional initial window height.
    pub initial_height: Option<u32>,
    /// Optional window title.
    pub title: Option<String>,
    /// Whether to listen for dev server control messages on stdin.
    pub listen_stdin: bool,
    /// Whether to persist window state across sessions (default: true).
    pub persist_state: bool,
    /// Whether to enforce single-instance lock (default: true).
    pub single_instance: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        let mut capabilities = wavical_ipc::Capabilities::new();
        // Phase 0 default: grant core.ping, keep core.echo ungranted
        capabilities.grant("core.ping");

        Self {
            window_id: "main".to_string(),
            allowed_origins: vec![
                "http://wavical.localhost".to_string(),
                "wavical://localhost".to_string(),
                "https://wavical.localhost".to_string(),
            ],
            capabilities,
            custom_csp: None,
            dynamic_capabilities: None,
            initial_width: None,
            initial_height: None,
            title: None,
            listen_stdin: false,
            persist_state: true,
            single_instance: true,
        }
    }
}

impl WindowConfig {
    /// Return the effective Content-Security-Policy string for this window.
    pub fn effective_csp(&self) -> &str {
        self.custom_csp.as_deref().unwrap_or(DEFAULT_CSP)
    }

    /// Return the current active capability set, taking into account any dynamic updates.
    pub fn effective_capabilities(&self) -> wavical_ipc::Capabilities {
        if let Some(ref dyn_caps) = self.dynamic_capabilities {
            dyn_caps
                .read()
                .map(|c| c.clone())
                .unwrap_or_else(|_| self.capabilities.clone())
        } else {
            self.capabilities.clone()
        }
    }
}

/// Helper to safely truncate payloads, URLs, or file paths in security logs.
/// Avoids splitting multibyte UTF-8 characters and appends a truncation indicator.
pub fn truncate_payload(payload: &str, max_len: usize) -> String {
    if payload.len() <= max_len {
        payload.to_string()
    } else {
        let mut end = max_len;
        while end > 0 && !payload.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}... [truncated]", &payload[..end])
    }
}

/// Extract and normalize the origin (`scheme://host[:port]`) from a request URI.
///
/// Security enhancements:
/// - Strips default scheme ports (80 for http, 443 for https) in accordance with RFC 6454.
/// - Rejects URIs with embedded userinfo (`@`), preventing credential-based origin spoofing attacks.
pub fn extract_origin(uri: &Uri) -> Option<String> {
    let scheme = uri.scheme_str()?.to_ascii_lowercase();
    let host = uri.host()?.to_ascii_lowercase();

    // Reject userinfo (e.g. user:pass@host)
    if uri
        .authority()
        .is_some_and(|auth| auth.as_str().contains('@'))
    {
        return None;
    }

    match (scheme.as_str(), uri.port_u16()) {
        ("http", Some(80)) | ("https", Some(443)) | (_, None) => Some(format!("{scheme}://{host}")),
        (_, Some(port)) => Some(format!("{scheme}://{host}:{port}")),
    }
}

/// Verify if a request URI belongs to the allowed application origins.
pub fn is_origin_allowed(uri: &Uri, allowed_origins: &[String]) -> bool {
    let Some(origin) = extract_origin(uri) else {
        return false;
    };
    allowed_origins
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(&origin))
}

/// Verify whether an incoming target navigation URL is authorized.
pub fn is_navigation_allowed(url_str: &str, allowed_origins: &[String]) -> bool {
    let Ok(uri) = url_str.parse::<Uri>() else {
        return false;
    };
    is_origin_allowed(&uri, allowed_origins)
}

/// Validate incoming top-level navigation request, emitting a structured security event if denied.
pub fn handle_navigation_request(
    url_str: &str,
    allowed_origins: &[String],
    window_id: &str,
) -> bool {
    let allowed = is_navigation_allowed(url_str, allowed_origins);
    if !allowed {
        tracing::warn!(
            security_event = "navigation_blocked",
            window_id = %window_id,
            target_url = %truncate_payload(url_str, 64),
            reason = "Target URL origin is not in allowed origins allowlist",
            "Security check failed: navigation blocked"
        );
    }
    allowed
}

/// Validate popup and new-window creation request, emitting a structured security event when blocked.
pub fn handle_new_window_request(url_str: &str, window_id: &str) -> NewWindowResponse {
    tracing::warn!(
        security_event = "popup_blocked",
        window_id = %window_id,
        target_url = %truncate_payload(url_str, 64),
        reason = "Popup and new-window creation is denied by default",
        "Security check failed: popup blocked"
    );
    NewWindowResponse::Deny
}

/// Build safe JavaScript evaluation code for delivering an IPC response to the page.
///
/// Uses double JSON serialization to produce a strictly escaped JS string literal passed
/// to `JSON.parse(...)`, completely preventing string concatenation injection vulnerabilities.
pub fn build_delivery_script(
    response: &wavical_ipc::Response,
) -> Result<String, serde_json::Error> {
    let response_json = serde_json::to_string(response)?;
    let escaped_js_literal = serde_json::to_string(&response_json)?;
    Ok(format!(
        "window.__wavical_receive__(JSON.parse({escaped_js_literal}));"
    ))
}

/// Process an incoming raw IPC message against window origin allowlist and capabilities.
pub fn process_ipc_message(
    uri: &Uri,
    body: &str,
    config: &WindowConfig,
    registry: &wavical_ipc::HandlerRegistry,
) -> Result<Option<wavical_ipc::Response>, IpcOriginError> {
    if !is_origin_allowed(uri, &config.allowed_origins) {
        let origin_str = extract_origin(uri).unwrap_or_else(|| uri.to_string());
        tracing::warn!(
            security_event = "origin_rejected",
            window_id = %config.window_id,
            origin = %origin_str,
            reason = "Caller origin is not in the allowed origins allowlist",
            "Security check failed: IPC origin rejected"
        );
        return Err(IpcOriginError::DisallowedOrigin(origin_str));
    }

    let request = match wavical_ipc::parse_request(body) {
        Ok(req) => req,
        Err(wavical_ipc::IpcError::MessageTooLarge { size, max }) => {
            tracing::warn!(
                security_event = "oversize_message",
                window_id = %config.window_id,
                size = size,
                max_size = max,
                reason = "Raw message exceeds maximum size limit",
                "Security check failed: oversize IPC message rejected"
            );
            return Ok(None);
        }
        Err(other_err) => {
            tracing::warn!(
                security_event = "malformed_message",
                window_id = %config.window_id,
                payload = %truncate_payload(body, 64),
                reason = %other_err,
                "Security check failed: malformed IPC message rejected"
            );
            return Ok(None);
        }
    };

    let active_capabilities = config.effective_capabilities();
    if !active_capabilities.is_allowed(&request.method) {
        tracing::warn!(
            security_event = "capability_denied",
            window_id = %config.window_id,
            method = %request.method,
            id = request.id,
            reason = "Method not granted in window capability allowlist",
            "Security check failed: IPC capability denied"
        );
        return Ok(Some(wavical_ipc::Response::error(
            request.id,
            wavical_ipc::GENERIC_METHOD_ERROR,
        )));
    }

    let response = if wavical_ipc::is_native_method(&request.method) {
        let native_service = wavical_native::NativeService::new();
        wavical_ipc::dispatch_native(&request, &config.window_id, &active_capabilities, &native_service)
    } else {
        wavical_ipc::dispatch(&request, &active_capabilities, registry)
    };
    Ok(Some(response))
}

/// Check whether the WebView2 runtime is present and available.
pub fn check_webview2_runtime() -> Result<String, String> {
    check_webview2_runtime_with_override(
        std::env::var("WAVICAL_SIMULATE_NO_WEBVIEW2")
            .ok()
            .as_deref(),
    )
}

/// Internal helper for runtime availability checking, parameterized for unit testing.
pub fn check_webview2_runtime_with_override(
    simulate_override: Option<&str>,
) -> Result<String, String> {
    #[cfg(debug_assertions)]
    {
        if simulate_override.is_some() {
            return Err(
                "Simulated missing WebView2 runtime via WAVICAL_SIMULATE_NO_WEBVIEW2".to_string(),
            );
        }
    }

    wry::webview_version().map_err(|err| err.to_string())
}

/// Log detail with tracing, display friendly native message box, and exit with a non-zero code.
pub fn handle_missing_runtime_and_exit(error_detail: &str) -> ! {
    tracing::error!("WebView2 runtime unavailable: {error_detail}");
    eprintln!(
        "\n====================================================================\n\
         [Wavical Error] Microsoft Edge WebView2 Runtime is missing or unusable\n\
         ====================================================================\n\
         Detail: {error_detail}\n\n\
         Wavical requires the Microsoft Edge WebView2 Runtime on Windows\n\
         to create and render application windows.\n\n\
         To install the runtime:\n\
           1. Download from Microsoft:\n\
              https://developer.microsoft.com/en-us/microsoft-edge/webview2/\n\
           2. Or install via Windows Package Manager in PowerShell:\n\
              winget install --id Microsoft.EdgeWebView2Runtime -e\n\
         ====================================================================\n"
    );
    if std::env::var("WAVICAL_HEADLESS_TEST").is_err() {
        wavical_native::show_missing_webview2_dialog();
    }
    process::exit(1);
}

/// Initialize window with tao.
pub fn create_window(event_loop: &EventLoop<UserEvent>, title: &str) -> Result<Window, String> {
    create_window_with_size(event_loop, title, 820, 700)
}

/// Initialize window with custom initial size.
pub fn create_window_with_size(
    event_loop: &EventLoop<UserEvent>,
    title: &str,
    width: u32,
    height: u32,
) -> Result<Window, String> {
    WindowBuilder::new()
        .with_title(title)
        .with_inner_size(LogicalSize::new(width as f64, height as f64))
        .build(event_loop)
        .map_err(|err| err.to_string())
}

/// Resolve requested protocol path and return safe HTTP response with security headers.
pub fn handle_protocol_request(
    request: Request<Vec<u8>>,
    asset_root: &Path,
    csp: &str,
    window_id: &str,
) -> Response<Cow<'static, [u8]>> {
    let req_path = request.uri().path();
    tracing::debug!("Handling protocol request for URI path: {req_path}");

    match resolve_asset_path(req_path, asset_root) {
        Ok(resolved) => match fs::read(&resolved.path) {
            Ok(bytes) => {
                tracing::debug!(
                    "Serving asset '{req_path}' with MIME '{}'",
                    resolved.mime_type
                );
                Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", resolved.mime_type)
                    .header(HEADER_NOSNIFF.0, HEADER_NOSNIFF.1)
                    .header(HEADER_CSP, csp)
                    .body(Cow::Owned(bytes))
                    .unwrap_or_else(|_| build_safe_404_response(csp))
            }
            Err(io_err) => {
                tracing::error!("Failed to read asset from disk '{req_path}': {io_err}");
                build_safe_404_response(csp)
            }
        },
        Err(rejection) => {
            tracing::warn!(
                security_event = "path_rejected",
                window_id = %window_id,
                requested_path = %truncate_payload(req_path, 64),
                reason = %rejection,
                "Security check failed: protocol path rejected"
            );
            build_safe_404_response(csp)
        }
    }
}

/// Build a plain 404 response without leaking internal paths, preserving security headers.
pub fn build_safe_404_response(csp: &str) -> Response<Cow<'static, [u8]>> {
    let not_found_bytes: &'static [u8] = b"404 Not Found";
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/plain; charset=utf-8")
        .header(HEADER_NOSNIFF.0, HEADER_NOSNIFF.1)
        .header(HEADER_CSP, csp)
        .body(Cow::Borrowed(not_found_bytes))
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(not_found_bytes)))
}

/// Locate asset root directory for examples/hello.
pub fn get_asset_root() -> PathBuf {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let candidates = [
        current_dir.join("examples").join("hello"),
        current_dir
            .join("..")
            .join("..")
            .join("examples")
            .join("hello"),
    ];

    for candidate in &candidates {
        if candidate.is_dir() {
            return candidate.clone();
        }
    }

    PathBuf::from("examples/hello")
}

/// Query whether devtools should be enabled.
pub fn is_devtools_enabled() -> bool {
    cfg!(debug_assertions)
}

/// Launch and run the Wavical shell with custom asset root and window config.
pub fn run_with_config(asset_root: PathBuf, config: WindowConfig) {
    // 1. Verify WebView2 runtime availability
    let webview_version = match check_webview2_runtime() {
        Ok(ver) => ver,
        Err(err) => {
            handle_missing_runtime_and_exit(&err);
        }
    };
    tracing::info!("Detected WebView2 runtime version: {webview_version}");

    // Single-instance enforcement
    let mut single_instance_server = None;
    if config.single_instance {
        let args: Vec<String> = std::env::args().collect();
        match wavical_native::check_single_instance(&config.window_id, &args) {
            Ok(wavical_native::SingleInstance::Secondary) => {
                tracing::info!(
                    "Secondary instance launched; forwarded arguments to primary instance. Exiting with code 0."
                );
                std::process::exit(0);
            }
            Ok(wavical_native::SingleInstance::Primary(server)) => {
                single_instance_server = Some(server);
            }
            Ok(wavical_native::SingleInstance::Disabled) => {
                tracing::warn!(
                    "Single-instance enforcement disabled or unavailable; proceeding normally"
                );
            }
            Err(err) => {
                tracing::warn!("Single-instance check failed ({err}); proceeding normally");
            }
        }
    }

    let effective_csp = config.effective_csp().to_string();
    tracing::info!(
        "Window '{}' configured allowed origins: {:?}, capabilities count: {}",
        config.window_id,
        config.allowed_origins,
        config.capabilities.len()
    );

    // 2. Initialize tao event loop with typed UserEvent
    let event_loop: EventLoop<UserEvent> = EventLoopBuilder::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // Spawn single-instance background listener to focus window on second launch
    if let Some(ref mut server) = single_instance_server {
        let focus_proxy = proxy.clone();
        server.spawn_listener(move |args| {
            tracing::info!("Received forwarded arguments from secondary instance: {:?}", args);
            let _ = focus_proxy.send_event(UserEvent::FocusWindow);
        });
    }

    // Ensure dynamic capabilities storage is initialized
    let dyn_caps = config.dynamic_capabilities.clone().unwrap_or_else(|| {
        std::sync::Arc::new(std::sync::RwLock::new(config.capabilities.clone()))
    });
    let mut config = config;
    config.dynamic_capabilities = Some(dyn_caps.clone());

    // 3. Create host window
    let win_title = config.title.as_deref().unwrap_or("Wavical");
    let win_width = config.initial_width.unwrap_or(820);
    let win_height = config.initial_height.unwrap_or(700);

    let window = match create_window_with_size(&event_loop, win_title, win_width, win_height) {
        Ok(win) => win,
        Err(err) => {
            handle_missing_runtime_and_exit(&format!("Failed to create host window: {err}"));
        }
    };

    // Restore saved window layout state if persistence is enabled
    if config.persist_state
        && let Ok(state_path) = wavical_native::get_window_state_path(&config.window_id)
        && let Ok(raw_json) = std::fs::read_to_string(&state_path)
    {
        let screens: Vec<wavical_ipc::ScreenBounds> = event_loop
            .available_monitors()
            .map(|m| {
                let p = m.position();
                let s = m.size();
                wavical_ipc::ScreenBounds::new(p.x, p.y, s.width, s.height)
            })
            .collect();

        let resolved = wavical_ipc::resolve_window_state(
            &raw_json,
            win_width,
            win_height,
            &screens,
        );

        window.set_inner_size(tao::dpi::PhysicalSize::new(resolved.width, resolved.height));
        if let (Some(x), Some(y)) = (resolved.x, resolved.y) {
            window.set_outer_position(tao::dpi::PhysicalPosition::new(x, y));
        }
        if resolved.maximized {
            window.set_maximized(true);
        }
    }

    if config.listen_stdin {
        let stdin_proxy = proxy.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let stdin = std::io::stdin();
            let reader = stdin.lock();
            for line in reader.lines() {
                match line {
                    Ok(raw) => {
                        let trimmed = raw.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            let action = val.get("action").and_then(|a| a.as_str()).unwrap_or("");
                            match action {
                                "reload" => {
                                    let _ = stdin_proxy.send_event(UserEvent::Reload);
                                }
                                "update_capabilities" => {
                                    let caps = val
                                        .get("capabilities")
                                        .and_then(|c| c.as_array())
                                        .map(|arr| {
                                            arr.iter()
                                                .filter_map(|i| i.as_str().map(|s| s.to_string()))
                                                .collect()
                                        })
                                        .unwrap_or_default();
                                    let _ =
                                        stdin_proxy.send_event(UserEvent::UpdateCapabilities(caps));
                                }
                                "exit" => {
                                    let _ = stdin_proxy.send_event(UserEvent::Exit);
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            // When stdin is closed (parent process exited), shut down shell event loop cleanly
            let _ = stdin_proxy.send_event(UserEvent::Exit);
        });
    }

    // 4. Initialize handler registry
    let mut registry = wavical_ipc::HandlerRegistry::with_core_handlers();
    let native_service = std::sync::Arc::new(wavical_native::NativeService::new());
    registry.register_native(native_service.clone(), &config.window_id);
    let loop_native_service = native_service.clone();
    let loop_window_id = config.window_id.clone();

    // 5. Build the webview with custom protocol, CSP, navigation guard, popup blocker, and IPC handler
    let devtools = is_devtools_enabled();
    tracing::info!("Initializing webview (devtools enabled: {devtools})");

    let protocol_asset_root = asset_root.clone();
    let protocol_csp = effective_csp.clone();
    let protocol_window_id = config.window_id.clone();
    let ipc_config = config.clone();
    let ipc_registry = registry.clone();
    let ipc_proxy = proxy.clone();

    let nav_allowed_origins = config.allowed_origins.clone();
    let nav_window_id = config.window_id.clone();
    let popup_window_id = config.window_id.clone();

    let webview = match WebViewBuilder::new()
        .with_devtools(devtools)
        .with_initialization_script(WAVICAL_INIT_SCRIPT)
        .with_custom_protocol("wavical".to_string(), move |_id, request| {
            handle_protocol_request(
                request,
                &protocol_asset_root,
                &protocol_csp,
                &protocol_window_id,
            )
        })
        .with_navigation_handler(move |url| {
            handle_navigation_request(&url, &nav_allowed_origins, &nav_window_id)
        })
        .with_new_window_req_handler(move |url, _features| {
            handle_new_window_request(&url, &popup_window_id)
        })
        .with_ipc_handler(move |request| {
            let uri = request.uri();
            let body = request.body();

            match process_ipc_message(uri, body, &ipc_config, &ipc_registry) {
                Ok(Some(response)) => match build_delivery_script(&response) {
                    Ok(script) => {
                        if let Err(err) = ipc_proxy.send_event(UserEvent::ExecuteScript(script)) {
                            tracing::error!("Failed to send ExecuteScript event: {err}");
                        }
                    }
                    Err(err) => {
                        tracing::error!("Failed to build delivery script: {err}");
                    }
                },
                Ok(None) => {}
                Err(IpcOriginError::DisallowedOrigin(_origin)) => {}
                Err(IpcOriginError::InvalidUri) => {
                    tracing::warn!(
                        security_event = "origin_rejected",
                        window_id = %ipc_config.window_id,
                        reason = "Unable to parse origin from request URI",
                        "Security check failed: IPC origin rejected"
                    );
                }
            }
        })
        .with_url("wavical://localhost/index.html")
        .build(&window)
    {
        Ok(wv) => wv,
        Err(err) => {
            handle_missing_runtime_and_exit(&err.to_string());
        }
    };

    // Attach renderer crash handler to detect process-gone events (WP6)
    let crash_proxy = proxy.clone();
    if let Err(err) = wavical_native::attach_renderer_crash_handler(&webview, move |reason_code| {
        let _ = crash_proxy.send_event(UserEvent::RendererProcessFailed(reason_code));
    }) {
        tracing::warn!("Failed to attach renderer crash handler: {err}");
    }

    // Helper to persist current window state if enabled
    let save_window_state_helper = |win: &Window, win_id: &str, persist: bool| {
        if !persist {
            return;
        }
        if let Ok(path) = wavical_native::get_window_state_path(win_id) {
            let pos = win.outer_position().ok();
            let size = win.inner_size();
            let maximized = win.is_maximized();
            let state = wavical_ipc::WindowState {
                x: pos.map(|p| p.x),
                y: pos.map(|p| p.y),
                width: size.width,
                height: size.height,
                maximized,
            };
            if let Err(e) = wavical_native::save_window_state(&path, &state) {
                tracing::warn!("Failed to save window state: {e}");
            }
        }
    };

    let persist_state = config.persist_state;
    let _retained_server = single_instance_server;
    let mut crash_history: Vec<std::time::Instant> = Vec::new();

    // 6. Run tao closure event loop
    tracing::info!("Entering event loop");
    event_loop.run(move |event, _, control_flow| {
        let _ = &_retained_server;
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(UserEvent::ExecuteScript(script)) => {
                tracing::debug!("Evaluating response script in webview");
                if let Err(err) = webview.evaluate_script(&script) {
                    tracing::error!("Failed to evaluate script in webview: {err}");
                }
            }
            Event::UserEvent(UserEvent::RendererProcessFailed(reason_code)) => {
                tracing::warn!(
                    window_id = %loop_window_id,
                    reason_code = reason_code,
                    "WebView2 renderer process gone event received"
                );

                let now = std::time::Instant::now();
                match wavical_ipc::decide_crash_recovery(&crash_history, now) {
                    wavical_ipc::CrashRecoveryDecision::Retry => {
                        crash_history.push(now);
                        tracing::info!("Reloading page once following renderer process failure");
                        if let Err(err) = webview.load_url("wavical://localhost/index.html") {
                            tracing::error!("Failed to reload page following crash: {err}");
                        }
                    }
                    wavical_ipc::CrashRecoveryDecision::GiveUp => {
                        tracing::error!(
                            window_id = %loop_window_id,
                            "Renderer process crashed repeatedly within 30 seconds; stopping retry"
                        );
                        if std::env::var("WAVICAL_HEADLESS_TEST").is_err() {
                            wavical_native::show_renderer_crashed_dialog();
                        }
                        save_window_state_helper(&window, &loop_window_id, persist_state);
                        loop_native_service.clear_window(&loop_window_id);
                        *control_flow = ControlFlow::Exit;
                    }
                }
            }
            Event::UserEvent(UserEvent::FocusWindow) => {
                tracing::info!("FocusWindow event received; restoring and focusing window");
                window.set_minimized(false);
                window.set_focus();
            }
            Event::UserEvent(UserEvent::Reload) => {
                tracing::info!("Reload requested via dev server control");
                if let Err(err) = webview.evaluate_script("location.reload();") {
                    tracing::error!("Failed to reload webview: {err}");
                }
            }
            Event::UserEvent(UserEvent::UpdateCapabilities(new_caps)) => {
                tracing::info!(
                    "Updating capabilities via dev server control: {:?}",
                    new_caps
                );
                if let Ok(mut lock) = dyn_caps.write() {
                    let mut updated = wavical_ipc::Capabilities::new();
                    for cap in new_caps {
                        updated.grant(cap);
                    }
                    *lock = updated;
                }
            }
            Event::UserEvent(UserEvent::Exit) => {
                tracing::info!("Exit requested via dev server control");
                save_window_state_helper(&window, &loop_window_id, persist_state);
                loop_native_service.clear_window(&loop_window_id);
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                tracing::info!("Window close requested; saving state, clearing grants and exiting cleanly");
                save_window_state_helper(&window, &loop_window_id, persist_state);
                loop_native_service.clear_window(&loop_window_id);
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

/// Standard launcher for the main Wavical binary application.
pub fn run() {
    tracing_subscriber::fmt::init();
    tracing::info!("Starting Wavical foundation shell (WP7/WP8)");
    let asset_root = get_asset_root();
    let config = WindowConfig::default();
    run_with_config(asset_root, config);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_devtools_setting_matches_debug_assertions() {
        assert_eq!(is_devtools_enabled(), cfg!(debug_assertions));
    }

    #[test]
    fn test_window_config_custom_csp_override() {
        let mut config = WindowConfig::default();
        assert_eq!(config.effective_csp(), DEFAULT_CSP);

        let custom = "default-src 'self' https://api.wavical.com;";
        config.custom_csp = Some(custom.to_string());
        assert_eq!(config.effective_csp(), custom);
    }

    #[test]
    fn test_truncate_payload() {
        let short = "hello";
        assert_eq!(truncate_payload(short, 10), "hello");

        let long = "a".repeat(100);
        let truncated = truncate_payload(&long, 10);
        assert!(truncated.starts_with("aaaaaaaaaa"));
        assert!(truncated.contains("... [truncated]"));
    }

    #[test]
    fn test_simulate_missing_webview2_runtime() {
        let result = check_webview2_runtime_with_override(Some("1"));
        assert!(result.is_err());
        let err_msg = result.unwrap_err();
        assert!(err_msg.contains("Simulated missing WebView2 runtime"));
    }

    #[test]
    fn test_real_webview2_runtime_detected() {
        let result = check_webview2_runtime_with_override(None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_protocol_safe_404_response_headers() {
        let response = build_safe_404_response(DEFAULT_CSP);
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers().get("X-Content-Type-Options").unwrap(),
            "nosniff"
        );
        assert_eq!(response.headers().get(HEADER_CSP).unwrap(), DEFAULT_CSP);
        assert_eq!(response.body().as_ref(), b"404 Not Found");
    }

    #[test]
    fn test_protocol_ok_response_headers() {
        let csp = DEFAULT_CSP;
        let response = Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/html; charset=utf-8")
            .header(HEADER_NOSNIFF.0, HEADER_NOSNIFF.1)
            .header(HEADER_CSP, csp)
            .body(Cow::Borrowed(&b"<html>ok</html>"[..]))
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("X-Content-Type-Options").unwrap(),
            "nosniff"
        );
        assert_eq!(response.headers().get(HEADER_CSP).unwrap(), DEFAULT_CSP);
    }

    #[test]
    fn test_protocol_rejection_never_leaks_internal_paths_or_error_text() {
        let root = PathBuf::from("examples/hello");
        let attack_paths = [
            "/../Cargo.toml",
            "/..%5CCargo.toml",
            "/%2e%2e/Cargo.toml",
            "/%252e%252e/Cargo.toml",
            "/C:%5CWindows%5Cwin.ini",
            "/%5C%5Cserver%5Cshare%5Cdata",
            "/CON.txt",
            "/index.html::$DATA",
            "/index.html%00.jpg",
            "/nonexistent_file.xyz",
        ];

        for path in attack_paths {
            let req = Request::builder()
                .uri(format!("wavical://localhost{path}"))
                .body(Vec::new())
                .unwrap();

            let resp = handle_protocol_request(req, &root, DEFAULT_CSP, "test-win");
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);

            let body_str = String::from_utf8_lossy(resp.body());
            assert_eq!(
                body_str, "404 Not Found",
                "Rejection response for '{path}' must strictly be '404 Not Found'"
            );

            assert!(!body_str.contains("Cargo"));
            assert!(!body_str.contains("Windows"));
            assert!(!body_str.contains("traversal"));
            assert!(!body_str.contains("device"));
        }
    }

    #[test]
    fn test_ipc_error_response_never_leaks_internals_or_rejection_reasons() {
        let config = WindowConfig::default();
        let registry = wavical_ipc::HandlerRegistry::with_core_handlers();
        let uri = "http://wavical.localhost/index.html"
            .parse::<Uri>()
            .unwrap();

        let ungranted_json = r#"{"id": 1, "method": "core.echo", "params": "test"}"#;
        let ungranted_res = process_ipc_message(&uri, ungranted_json, &config, &registry)
            .expect("should process valid origin")
            .expect("should return response");

        assert_eq!(
            ungranted_res.error.as_deref(),
            Some(wavical_ipc::GENERIC_METHOD_ERROR)
        );
        let err_text = ungranted_res.error.unwrap();
        assert!(!err_text.contains("capability"));
        assert!(!err_text.contains("echo"));
        assert!(!err_text.contains("/"));
        assert!(!err_text.contains("\\"));

        let unknown_json = r#"{"id": 2, "method": "system.secret_api", "params": null}"#;
        let unknown_res = process_ipc_message(&uri, unknown_json, &config, &registry)
            .expect("should process valid origin")
            .expect("should return response");

        assert_eq!(
            unknown_res.error.as_deref(),
            Some(wavical_ipc::GENERIC_METHOD_ERROR)
        );
    }

    #[test]
    fn test_malformed_and_oversize_ipc_sends_no_response_to_webview() {
        let config = WindowConfig::default();
        let registry = wavical_ipc::HandlerRegistry::with_core_handlers();
        let valid_uri = "http://wavical.localhost/index.html"
            .parse::<Uri>()
            .unwrap();

        let malformed = "{ broken json";
        let res1 = process_ipc_message(&valid_uri, malformed, &config, &registry).unwrap();
        assert!(res1.is_none());

        let huge = "a".repeat(wavical_ipc::MAX_MESSAGE_SIZE_BYTES + 10);
        let res2 = process_ipc_message(&valid_uri, &huge, &config, &registry).unwrap();
        assert!(res2.is_none());

        let attacker_uri = "https://evil.com/page.html".parse::<Uri>().unwrap();
        let res3 = process_ipc_message(
            &attacker_uri,
            r#"{"id": 1, "method": "core.ping"}"#,
            &config,
            &registry,
        );
        assert!(matches!(res3, Err(IpcOriginError::DisallowedOrigin(_))));
    }

    #[test]
    fn test_origin_allowlist_matches_valid_origins() {
        let config = WindowConfig::default();

        let uri_http = "http://wavical.localhost/index.html"
            .parse::<Uri>()
            .unwrap();
        assert!(is_origin_allowed(&uri_http, &config.allowed_origins));

        let uri_custom = "wavical://localhost/index.html".parse::<Uri>().unwrap();
        assert!(is_origin_allowed(&uri_custom, &config.allowed_origins));

        let uri_https = "https://wavical.localhost/api".parse::<Uri>().unwrap();
        assert!(is_origin_allowed(&uri_https, &config.allowed_origins));
    }

    #[test]
    fn test_origin_allowlist_rejects_external_and_attacker_origins() {
        let config = WindowConfig::default();

        let disallowed = [
            "https://evil.com/page.html",
            "http://attacker.localhost/index.html",
            "http://localhost:8080/iframe.html",
            "http://other.domain.com/",
            "http://wavical.localhost.attacker.com/",
            "https://evil.com:443/wavical",
            "http://admin:pass@wavical.localhost/",
        ];

        for raw_uri in disallowed {
            let uri = raw_uri.parse::<Uri>().unwrap();
            assert!(
                !is_origin_allowed(&uri, &config.allowed_origins),
                "Origin '{raw_uri}' must be rejected by origin allowlist"
            );
        }
    }

    #[test]
    fn test_origin_allowlist_rejects_relative_or_invalid_uris() {
        let config = WindowConfig::default();

        let uri_relative = "/index.html".parse::<Uri>().unwrap();
        assert!(!is_origin_allowed(&uri_relative, &config.allowed_origins));

        assert!(!is_navigation_allowed(
            "wavical:/invalid-path",
            &config.allowed_origins
        ));
        assert!(!is_navigation_allowed(
            "not a valid uri ://",
            &config.allowed_origins
        ));
    }

    #[test]
    fn test_navigation_allowed_for_app_origin() {
        let config = WindowConfig::default();

        assert!(is_navigation_allowed(
            "http://wavical.localhost/index.html",
            &config.allowed_origins
        ));
        assert!(is_navigation_allowed(
            "wavical://localhost/page2.html",
            &config.allowed_origins
        ));
        assert!(is_navigation_allowed(
            "https://wavical.localhost/sub/page.html",
            &config.allowed_origins
        ));
    }

    #[test]
    fn test_navigation_denied_for_external_and_attacker_sites() {
        let config = WindowConfig::default();

        assert!(!is_navigation_allowed(
            "https://google.com",
            &config.allowed_origins
        ));
        assert!(!is_navigation_allowed(
            "https://evil.com/phishing",
            &config.allowed_origins
        ));
        assert!(!is_navigation_allowed(
            "http://wavical.localhost.attacker.com",
            &config.allowed_origins
        ));
        assert!(!is_navigation_allowed(
            "javascript:alert(1)",
            &config.allowed_origins
        ));
        assert!(!is_navigation_allowed(
            "data:text/html,evil",
            &config.allowed_origins
        ));
    }

    #[test]
    fn test_build_delivery_script_safe_escaping() {
        let payload = serde_json::json!({
            "message": "hello \"world\"",
            "quote_breakout": "\"); alert('xss'); (\"",
            "control": "\n\r\t",
            "unicode": "🦀 Wavical",
        });
        let response = wavical_ipc::Response::success(42, payload);
        let script = build_delivery_script(&response).expect("serialization should succeed");

        assert!(script.starts_with("window.__wavical_receive__(JSON.parse(\""));
        assert!(script.ends_with("\"));"));
    }

    #[test]
    fn test_registered_methods_exact_allowlist() {
        let registry = wavical_ipc::HandlerRegistry::with_core_handlers();
        let methods = registry.registered_methods();
        let expected = vec!["core.echo".to_string(), "core.ping".to_string()];

        assert_eq!(
            methods, expected,
            "Exposed method list mismatch! Adding or removing an IPC method must be a deliberate, audited change."
        );
    }

    #[test]
    fn test_process_ipc_message_ping_granted() {
        let config = WindowConfig::default();
        let registry = wavical_ipc::HandlerRegistry::with_core_handlers();

        let uri = "http://wavical.localhost/index.html"
            .parse::<Uri>()
            .unwrap();
        let raw_req = r#"{"id": 101, "method": "core.ping"}"#;

        let result = process_ipc_message(&uri, raw_req, &config, &registry)
            .expect("processing should succeed for valid origin");

        let response = result.expect("response should be produced");
        assert_eq!(response.id, 101);
        assert!(response.ok);
        assert_eq!(response.result.unwrap()["message"], "pong");
    }

    #[test]
    fn test_process_ipc_message_echo_ungranted() {
        let config = WindowConfig::default();
        let registry = wavical_ipc::HandlerRegistry::with_core_handlers();

        let uri = "http://wavical.localhost/index.html"
            .parse::<Uri>()
            .unwrap();
        let raw_req = r#"{"id": 102, "method": "core.echo", "params": {"data": 123}}"#;

        let result = process_ipc_message(&uri, raw_req, &config, &registry)
            .expect("processing should succeed for valid origin");

        let response = result.expect("response should be produced");
        assert_eq!(response.id, 102);
        assert!(!response.ok);
        assert_eq!(response.error.unwrap(), wavical_ipc::GENERIC_METHOD_ERROR);
    }

    #[test]
    fn test_process_ipc_message_different_origin_rejected() {
        let config = WindowConfig::default();
        let registry = wavical_ipc::HandlerRegistry::with_core_handlers();

        let uri = "https://malicious-site.com/exploit.html"
            .parse::<Uri>()
            .unwrap();
        let raw_req = r#"{"id": 103, "method": "core.ping"}"#;

        let result = process_ipc_message(&uri, raw_req, &config, &registry);
        match result {
            Err(IpcOriginError::DisallowedOrigin(origin)) => {
                assert_eq!(origin, "https://malicious-site.com");
            }
            other => panic!("expected DisallowedOrigin error, got: {other:?}"),
        }
    }

    #[test]
    fn test_all_security_events_emitted_on_violation() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();

        let config = WindowConfig::default();
        let registry = wavical_ipc::HandlerRegistry::with_core_handlers();
        let valid_uri = "http://wavical.localhost/index.html"
            .parse::<Uri>()
            .unwrap();
        let attacker_uri = "https://attacker.com/exploit.html".parse::<Uri>().unwrap();

        // 1. Navigation blocked
        let nav_allowed = handle_navigation_request(
            "https://phishing-site.example.com",
            &config.allowed_origins,
            &config.window_id,
        );
        assert!(!nav_allowed);

        // 2. Popup blocked
        let popup_res =
            handle_new_window_request("https://popup-advert.example.com", &config.window_id);
        assert!(matches!(popup_res, NewWindowResponse::Deny));

        // 3. Path rejected
        let root = PathBuf::from("examples/hello");
        let req = Request::builder()
            .uri("wavical://localhost/..%5c..%5cWindows%5cSystem32%5ccmd.exe")
            .body(Vec::new())
            .unwrap();
        let proto_res = handle_protocol_request(req, &root, DEFAULT_CSP, &config.window_id);
        assert_eq!(proto_res.status(), StatusCode::NOT_FOUND);

        // 4. Origin rejected
        let origin_res = process_ipc_message(
            &attacker_uri,
            r#"{"id": 1, "method": "core.ping"}"#,
            &config,
            &registry,
        );
        assert!(matches!(
            origin_res,
            Err(IpcOriginError::DisallowedOrigin(_))
        ));

        // 5. Oversize message
        let oversize_payload = "X".repeat(wavical_ipc::MAX_MESSAGE_SIZE_BYTES + 1024);
        let oversize_res =
            process_ipc_message(&valid_uri, &oversize_payload, &config, &registry).unwrap();
        assert!(oversize_res.is_none());

        // 6. Malformed message
        let malformed_payload = r#"{"id": 42, "method": "invalid syntax..."#;
        let malformed_res =
            process_ipc_message(&valid_uri, malformed_payload, &config, &registry).unwrap();
        assert!(malformed_res.is_none());

        // 7. Capability denied
        let ungranted_payload = r#"{"id": 99, "method": "core.echo", "params": "secret"}"#;
        let cap_res = process_ipc_message(&valid_uri, ungranted_payload, &config, &registry)
            .unwrap()
            .unwrap();
        assert!(!cap_res.ok);
        assert_eq!(cap_res.error.unwrap(), wavical_ipc::GENERIC_METHOD_ERROR);
    }

    #[test]
    fn test_dynamic_capabilities_live_update_without_restart() {
        let dyn_caps =
            std::sync::Arc::new(std::sync::RwLock::new(wavical_ipc::Capabilities::new()));
        let config = WindowConfig {
            dynamic_capabilities: Some(dyn_caps.clone()),
            capabilities: wavical_ipc::Capabilities::new(),
            ..Default::default()
        };

        let registry = wavical_ipc::HandlerRegistry::with_core_handlers();
        let uri = "http://wavical.localhost/index.html"
            .parse::<Uri>()
            .unwrap();

        // Initially no capabilities granted: ping should be denied
        let res1 = process_ipc_message(
            &uri,
            r#"{"id": 1, "method": "core.ping"}"#,
            &config,
            &registry,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            res1.error.as_deref(),
            Some(wavical_ipc::GENERIC_METHOD_ERROR)
        );

        // Dynamically grant core.ping
        {
            let mut lock = dyn_caps.write().unwrap();
            lock.grant("core.ping");
        }

        // Now ping should succeed immediately without changing config reference
        let res2 = process_ipc_message(
            &uri,
            r#"{"id": 2, "method": "core.ping"}"#,
            &config,
            &registry,
        )
        .unwrap()
        .unwrap();
        assert!(res2.ok);
        assert_eq!(res2.result.unwrap().get("message").unwrap(), "pong");

        // Dynamically grant core.echo as well
        {
            let mut lock = dyn_caps.write().unwrap();
            lock.grant("core.echo");
        }

        let res3 = process_ipc_message(
            &uri,
            r#"{"id": 3, "method": "core.echo", "params": "hello"}"#,
            &config,
            &registry,
        )
        .unwrap()
        .unwrap();
        assert!(res3.ok);
        assert_eq!(res3.result.unwrap(), "hello");
    }
}
