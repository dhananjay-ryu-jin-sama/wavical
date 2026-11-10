#![forbid(unsafe_code)]

//! Pure argument validation, permission flag construction, and environment sanitization for the backend runtime.
//!
//! Enforces:
//! - Complete absence of `--inspect`, `--inspect-brk`, `--require`, `--import`, `--loader` in release builds.
//! - Deterministic construction of Node.js permission model flags.
//! - Filtering of environment variables to prevent injection via `NODE_OPTIONS`, `NODE_PATH`, etc.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum BackendArgError {
    #[error("debug/preload flag '{0}' is strictly forbidden in release builds")]
    DebugFlagForbiddenInRelease(String),
}

/// Check if a CLI argument enables debugging, inspector access, or code preloading.
pub fn is_forbidden_backend_arg(arg: &str) -> bool {
    let lower = arg.to_lowercase();
    let trimmed = lower.trim();

    if trimmed == "--inspect"
        || trimmed.starts_with("--inspect=")
        || trimmed == "--inspect-brk"
        || trimmed.starts_with("--inspect-brk=")
        || trimmed == "--inspect-port"
        || trimmed.starts_with("--inspect-port=")
        || trimmed == "--debug"
        || trimmed.starts_with("--debug=")
        || trimmed == "--debug-brk"
        || trimmed.starts_with("--debug-brk=")
        || trimmed == "--debug-port"
        || trimmed.starts_with("--debug-port=")
        || trimmed == "--require"
        || trimmed.starts_with("--require=")
        || trimmed == "-r"
        || trimmed == "--import"
        || trimmed.starts_with("--import=")
        || trimmed == "--loader"
        || trimmed.starts_with("--loader=")
        || trimmed == "--experimental-loader"
        || trimmed.starts_with("--experimental-loader=")
    {
        return true;
    }

    false
}

/// Validate a list of CLI arguments for the backend process.
/// In release builds (`is_release = true`), refuses to start if any debugging or preloading flags are present.
pub fn validate_backend_cli_args(args: &[String], is_release: bool) -> Result<(), BackendArgError> {
    if is_release {
        for arg in args {
            if is_forbidden_backend_arg(arg) {
                return Err(BackendArgError::DebugFlagForbiddenInRelease(arg.clone()));
            }
        }
    }
    Ok(())
}

/// Check if an environment variable name is forbidden for Node.js child processes.
///
/// Forbids:
/// - `NODE_OPTIONS`
/// - `NODE_PATH`
/// - `NODE_ENV`
/// - `NODE_DEBUG` / `NODE_DEBUG_NATIVE`
/// - `NODE_V8_COVERAGE`
/// - `NODE_EXTRA_CA_CERTS`
/// - `NODE_TLS_REJECT_UNAUTHORIZED`
/// - Any variable starting with `NODE_` that could alter runtime behavior or debugging.
pub fn is_forbidden_node_env_var(name: &str) -> bool {
    let upper = name.to_uppercase();
    let trimmed = upper.trim();

    if trimmed == "NODE_OPTIONS"
        || trimmed == "NODE_PATH"
        || trimmed == "NODE_ENV"
        || trimmed == "NODE_DEBUG"
        || trimmed == "NODE_DEBUG_NATIVE"
        || trimmed == "NODE_V8_COVERAGE"
        || trimmed == "NODE_EXTRA_CA_CERTS"
        || trimmed == "NODE_TLS_REJECT_UNAUTHORIZED"
        || trimmed == "NODE_REDIRECT_WARNINGS"
        || trimmed == "NODE_PENDING_DEPRECATION"
        || trimmed == "NODE_PRESERVE_SYMLINKS"
        || trimmed == "NODE_PRESERVE_SYMLINKS_MAIN"
    {
        return true;
    }

    // Any variable starting with NODE_INSPECT or NODE_DEBUG
    if trimmed.starts_with("NODE_INSPECT") || trimmed.starts_with("NODE_DEBUG") {
        return true;
    }

    false
}

/// Build the permission model arguments for Node.js given allowed roots and capabilities.
///
/// If `unrestricted` is true, returns an empty list (permission model disabled).
/// Otherwise, outputs:
/// - `--experimental-permission` (for Node < 23) or `--permission` (for Node >= 23)
/// - `--allow-fs-read=<root>` for each read root
/// - `--allow-fs-write=<root>` for each write root
/// - `--allow-child-process` if child process permission is granted
/// - `--allow-worker` if worker threads permission is granted
pub fn build_node_permission_flags(
    unrestricted: bool,
    major_version: u32,
    read_roots: &[String],
    write_roots: &[String],
    allow_child_process: bool,
    allow_workers: bool,
) -> Vec<String> {
    if unrestricted {
        return Vec::new();
    }

    let mut flags = Vec::new();

    if major_version >= 23 {
        flags.push("--permission".to_string());
    } else {
        flags.push("--experimental-permission".to_string());
    }

    for root in read_roots {
        let trimmed = root.trim();
        if !trimmed.is_empty() {
            flags.push(format!("--allow-fs-read={trimmed}"));
        }
    }

    for root in write_roots {
        let trimmed = root.trim();
        if !trimmed.is_empty() {
            flags.push(format!("--allow-fs-write={trimmed}"));
        }
    }

    if allow_child_process {
        flags.push("--allow-child-process".to_string());
    }

    if allow_workers {
        flags.push("--allow-worker".to_string());
    }

    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_forbidden_backend_arg() {
        assert!(is_forbidden_backend_arg("--inspect"));
        assert!(is_forbidden_backend_arg("--inspect=9229"));
        assert!(is_forbidden_backend_arg("--inspect-brk"));
        assert!(is_forbidden_backend_arg("--inspect-brk=0.0.0.0:9229"));
        assert!(is_forbidden_backend_arg("--inspect-port=9229"));
        assert!(is_forbidden_backend_arg("--require=./evil.js"));
        assert!(is_forbidden_backend_arg("-r"));
        assert!(is_forbidden_backend_arg("--import=./evil.js"));
        assert!(is_forbidden_backend_arg("--loader=./loader.js"));
        assert!(is_forbidden_backend_arg(
            "--experimental-loader=./loader.js"
        ));

        // Allowed runtime flags
        assert!(!is_forbidden_backend_arg("--permission"));
        assert!(!is_forbidden_backend_arg("--experimental-permission"));
        assert!(!is_forbidden_backend_arg("--allow-fs-read=."));
        assert!(!is_forbidden_backend_arg("backend/main.js"));
    }

    #[test]
    fn test_validate_backend_cli_args_release_vs_debug() {
        let bad_args = vec!["--inspect".to_string(), "backend/main.js".to_string()];
        // In debug mode, does not error out
        assert!(validate_backend_cli_args(&bad_args, false).is_ok());

        // In release mode, strictly refused
        let err = validate_backend_cli_args(&bad_args, true).unwrap_err();
        assert!(matches!(
            err,
            BackendArgError::DebugFlagForbiddenInRelease(_)
        ));

        let good_args = vec![
            "--experimental-permission".to_string(),
            "--allow-fs-read=backend".to_string(),
            "backend/main.js".to_string(),
        ];
        assert!(validate_backend_cli_args(&good_args, true).is_ok());
    }

    #[test]
    fn test_is_forbidden_node_env_var() {
        assert!(is_forbidden_node_env_var("NODE_OPTIONS"));
        assert!(is_forbidden_node_env_var("node_options"));
        assert!(is_forbidden_node_env_var("NODE_PATH"));
        assert!(is_forbidden_node_env_var("NODE_ENV"));
        assert!(is_forbidden_node_env_var("NODE_DEBUG"));
        assert!(is_forbidden_node_env_var("NODE_INSPECT_RESUME_ON_START"));
        assert!(is_forbidden_node_env_var("NODE_V8_COVERAGE"));

        // Safe variables
        assert!(!is_forbidden_node_env_var("PORT"));
        assert!(!is_forbidden_node_env_var("DATABASE_URL"));
        assert!(!is_forbidden_node_env_var("MY_APP_SECRET"));
    }

    #[test]
    fn test_build_node_permission_flags_node_22() {
        let read_roots = vec!["C:/app/backend".to_string(), "C:/app/data".to_string()];
        let write_roots = vec!["C:/app/data".to_string()];

        let flags = build_node_permission_flags(false, 22, &read_roots, &write_roots, false, false);

        assert_eq!(flags[0], "--experimental-permission");
        assert_eq!(flags[1], "--allow-fs-read=C:/app/backend");
        assert_eq!(flags[2], "--allow-fs-read=C:/app/data");
        assert_eq!(flags[3], "--allow-fs-write=C:/app/data");
        assert_eq!(flags.len(), 4);
    }

    #[test]
    fn test_build_node_permission_flags_node_26_with_child_and_worker() {
        let read_roots = vec!["/backend".to_string()];
        let write_roots = vec!["/data".to_string()];

        let flags = build_node_permission_flags(false, 26, &read_roots, &write_roots, true, true);

        assert_eq!(flags[0], "--permission");
        assert_eq!(flags[1], "--allow-fs-read=/backend");
        assert_eq!(flags[2], "--allow-fs-write=/data");
        assert_eq!(flags[3], "--allow-child-process");
        assert_eq!(flags[4], "--allow-worker");
        assert_eq!(flags.len(), 5);
    }

    #[test]
    fn test_build_node_permission_flags_unrestricted_is_empty() {
        let read_roots = vec!["/backend".to_string()];
        let write_roots = vec!["/data".to_string()];

        let flags = build_node_permission_flags(true, 22, &read_roots, &write_roots, true, true);

        assert!(flags.is_empty());
    }
}
