#![forbid(unsafe_code)]

use std::process::{Command, exit};
use thiserror::Error;
use wavical_config::BackendConfig;
use wavical_protocol::{is_forbidden_node_env_var, validate_backend_cli_args};

/// Compile-time devtools setting: strictly `false` in release builds, `true` in debug builds.
pub const DEVTOOLS_COMPILED_ENABLED: bool = cfg!(debug_assertions);

/// WebView2 environment variable that can enable CDP remote debugging (`--remote-debugging-port`).
pub const WEBVIEW2_DEBUG_ENV_VAR: &str = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";

/// Internal sentinel set on the relaunched child process after stripping dangerous env vars.
pub const ENV_CLEANED_SENTINEL: &str = "WAVICAL_ENV_CLEANED";

/// Error returned when a release-mode startup security self-check fails.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StartupSelfCheckError {
    #[error("release startup self-check failed: {0}")]
    Failed(String),
}

/// Structured report of all release-mode hardening self-checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupSelfCheckReport {
    pub is_release: bool,
    pub devtools_disabled: bool,
    pub webview2_debug_env_clean: bool,
    pub node_debug_env_clean: bool,
    pub backend_config_clean: bool,
}

impl StartupSelfCheckReport {
    /// Returns `true` when every check passed.
    pub fn all_passed(&self) -> bool {
        self.devtools_disabled
            && self.webview2_debug_env_clean
            && self.node_debug_env_clean
            && self.backend_config_clean
    }
}

/// Scan the current process environment and return any variable names that
/// enable WebView2 remote debugging or Node.js debug/preload surfaces.
/// Only variable *names* are returned (never values).
pub fn find_dangerous_env_vars_in_process() -> Vec<String> {
    let mut found = Vec::new();
    for (k, _) in std::env::vars_os() {
        let name = k.to_string_lossy();
        if name.eq_ignore_ascii_case(WEBVIEW2_DEBUG_ENV_VAR) || is_forbidden_node_env_var(&name) {
            found.push(name.into_owned());
        }
    }
    found
}

/// In release builds, neutralize WebView2 remote-debugging and Node debug/preload
/// environment variables by relaunching the current executable with those variables
/// removed via `Command::env_remove` (avoiding `unsafe` `std::env::remove_var` in
/// Rust 2024 edition).
///
/// If dangerous variables are still present after relaunch (i.e. `WAVICAL_ENV_CLEANED`
/// is already set), `run_startup_self_check` will refuse to start.
pub fn neutralize_release_debug_env_or_relaunch() {
    if cfg!(debug_assertions) {
        return;
    }

    if std::env::var_os(ENV_CLEANED_SENTINEL).is_some() {
        return;
    }

    let dangerous = find_dangerous_env_vars_in_process();
    if dangerous.is_empty() {
        return;
    }

    tracing::warn!(
        target: "wavical.security",
        event = "release_debug_env_neutralized",
        var_count = dangerous.len(),
        vars = ?dangerous,
        "Stripping dangerous debug environment variables via clean relaunch"
    );

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("error: failed to locate current executable for env sanitization: {err}");
            exit(1);
        }
    };

    let mut cmd = Command::new(&exe);
    cmd.args(std::env::args_os().skip(1));
    cmd.env(ENV_CLEANED_SENTINEL, "1");
    for var in &dangerous {
        cmd.env_remove(var);
    }

    match cmd.status() {
        Ok(status) => exit(status.code().unwrap_or(1)),
        Err(err) => {
            eprintln!("error: failed to relaunch with sanitized environment: {err}");
            exit(1);
        }
    }
}

/// Evaluate all startup hardening checks and emit one structured tracing event
/// listing each check and its result.
pub fn evaluate_startup_self_check(
    is_release: bool,
    devtools_enabled: bool,
    env_keys: &[&str],
    backend_env: &[String],
    backend_args: &[String],
) -> Result<StartupSelfCheckReport, StartupSelfCheckError> {
    let devtools_disabled = !is_release || (!devtools_enabled && !DEVTOOLS_COMPILED_ENABLED);
    let webview2_debug_env_clean = !is_release
        || !env_keys
            .iter()
            .any(|k| k.eq_ignore_ascii_case(WEBVIEW2_DEBUG_ENV_VAR));
    let node_debug_env_clean =
        !is_release || !env_keys.iter().any(|k| is_forbidden_node_env_var(k));
    let backend_config_clean = !is_release
        || (!backend_env.iter().any(|k| is_forbidden_node_env_var(k))
            && validate_backend_cli_args(backend_args, is_release).is_ok());

    let report = StartupSelfCheckReport {
        is_release,
        devtools_disabled,
        webview2_debug_env_clean,
        node_debug_env_clean,
        backend_config_clean,
    };

    let passed = report.all_passed();
    tracing::info!(
        target: "wavical.security",
        event = "startup_self_check",
        is_release = report.is_release,
        devtools_disabled = report.devtools_disabled,
        webview2_debug_env_clean = report.webview2_debug_env_clean,
        node_debug_env_clean = report.node_debug_env_clean,
        backend_config_clean = report.backend_config_clean,
        passed = passed,
        "Startup security self-check completed"
    );

    if !passed {
        let mut reasons = Vec::new();
        if !report.devtools_disabled {
            reasons.push("devtools enabled in release build");
        }
        if !report.webview2_debug_env_clean {
            reasons.push("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS present in release environment");
        }
        if !report.node_debug_env_clean {
            reasons.push("forbidden Node.js debug/preload variable present in release environment");
        }
        if !report.backend_config_clean {
            reasons.push("forbidden Node.js debug/preload flag or variable in backend config");
        }
        return Err(StartupSelfCheckError::Failed(reasons.join("; ")));
    }

    Ok(report)
}

/// Run startup self-checks against the current process environment and optional `BackendConfig`.
pub fn run_startup_self_check(
    devtools_enabled: bool,
    backend_config: Option<&BackendConfig>,
) -> Result<StartupSelfCheckReport, StartupSelfCheckError> {
    let owned_keys: Vec<String> = std::env::vars_os()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    let key_refs: Vec<&str> = owned_keys.iter().map(String::as_str).collect();
    let backend_env = backend_config
        .filter(|b| b.enabled)
        .map(|b| b.env.as_slice())
        .unwrap_or(&[]);

    evaluate_startup_self_check(
        !cfg!(debug_assertions),
        devtools_enabled,
        &key_refs,
        backend_env,
        &[],
    )
}
