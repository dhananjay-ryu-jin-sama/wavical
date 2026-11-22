#![forbid(unsafe_code)]

//! Development-only command channel, startup token handshake, and manifest guard
//! for `wavical-shell-dev`. Never compiled into the production `wavical-shell` binary.

use std::io::{BufRead, BufReader, Stdin, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Unique binary marker present only in `wavical-shell-dev` (`#[cfg(feature = "dev-shell")]`).
/// Checked negatively against production `wavical-shell.exe` by package verification and audit tests.
pub const DEV_SHELL_BINARY_MARKER: &str = "WAVICAL_DEV_SHELL_CHANNEL_V1_MARKER";

/// Environment variable carrying the parent-provided dev session token.
pub const DEV_SESSION_TOKEN_ENV: &str = "WAVICAL_DEV_SESSION_TOKEN";

/// Maximum allowed length (in bytes) for a single line-delimited JSON dev command.
pub const MAX_DEV_COMMAND_BYTES: usize = 64 * 1024;

/// Maximum number of capabilities accepted in a `reload-config` command.
pub const MAX_DEV_CAPABILITIES_COUNT: usize = 256;

/// Timeout waiting for the parent CLI's initial `auth` command on stdin.
pub const DEV_AUTH_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Validated dev command received from the CLI over stdin.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "cmd", rename_all = "kebab-case", deny_unknown_fields)]
pub enum DevCommand {
    /// Initial handshake authenticating the stdin command pipe with the session token.
    Auth { id: u64, token: String },
    /// Reload the current webview page.
    Reload { id: u64, token: String },
    /// Apply updated capabilities and (where the mode allows) CSP/policy for the next page load.
    #[serde(rename_all = "camelCase")]
    ReloadConfig {
        id: u64,
        token: String,
        capabilities: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        network_mode: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remote_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dev_url: Option<String>,
    },
    /// Restart the Node.js backend process using the Phase 4 restart limiter.
    RestartBackend { id: u64, token: String },
    /// Exit the dev shell cleanly.
    Exit { id: u64, token: String },
}

impl DevCommand {
    /// Return the request correlation ID.
    pub fn id(&self) -> u64 {
        match self {
            Self::Auth { id, .. }
            | Self::Reload { id, .. }
            | Self::ReloadConfig { id, .. }
            | Self::RestartBackend { id, .. }
            | Self::Exit { id, .. } => *id,
        }
    }

    /// Return the session token supplied with this command.
    pub fn token(&self) -> &str {
        match self {
            Self::Auth { token, .. }
            | Self::Reload { token, .. }
            | Self::ReloadConfig { token, .. }
            | Self::RestartBackend { token, .. }
            | Self::Exit { token, .. } => token,
        }
    }
}

/// Reply written as a single JSON line to stdout after executing a `DevCommand`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevCommandReply {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub elapsed_ms: u64,
}

impl DevCommandReply {
    /// Construct a successful reply.
    pub fn ok(id: u64, elapsed_ms: u64) -> Self {
        Self {
            id,
            ok: true,
            error: None,
            elapsed_ms,
        }
    }

    /// Construct an error reply.
    pub fn err(id: u64, error: impl Into<String>, elapsed_ms: u64) -> Self {
        Self {
            id,
            ok: false,
            error: Some(error.into()),
            elapsed_ms,
        }
    }
}

/// Errors returned when parsing or validating a dev command line.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DevCommandParseError {
    #[error("dev command exceeds maximum size of {max} bytes (got {size} bytes)")]
    TooLarge { size: usize, max: usize },

    #[error("malformed dev command: {0}")]
    Malformed(String),

    #[error("invalid or unknown dev command: {0}")]
    Invalid(String),
}

/// Constant-time byte-slice equality to avoid timing leaks on session tokens.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Parse and strictly validate a single line-delimited JSON dev command.
/// Enforces size limit, known commands only, and `deny_unknown_fields`.
pub fn parse_dev_command(raw_line: &str) -> Result<DevCommand, DevCommandParseError> {
    if raw_line.len() > MAX_DEV_COMMAND_BYTES {
        return Err(DevCommandParseError::TooLarge {
            size: raw_line.len(),
            max: MAX_DEV_COMMAND_BYTES,
        });
    }

    let trimmed = raw_line.trim();
    if trimmed.is_empty() {
        return Err(DevCommandParseError::Malformed(
            "empty command line".to_string(),
        ));
    }

    let cmd: DevCommand = serde_json::from_str(trimmed).map_err(|err| {
        let msg = err.to_string();
        if msg.contains("unknown variant") || msg.contains("unknown field") {
            DevCommandParseError::Invalid(msg)
        } else {
            DevCommandParseError::Malformed(msg)
        }
    })?;

    if cmd.token().trim().is_empty() {
        return Err(DevCommandParseError::Invalid(
            "missing or empty session token".to_string(),
        ));
    }

    if let DevCommand::ReloadConfig {
        ref capabilities, ..
    } = cmd
        && capabilities.len() > MAX_DEV_CAPABILITIES_COUNT
    {
        return Err(DevCommandParseError::Invalid(format!(
            "capabilities list length {} exceeds maximum of {}",
            capabilities.len(),
            MAX_DEV_CAPABILITIES_COUNT
        )));
    }

    Ok(cmd)
}

/// Best-effort extraction of numeric `"id"` from a rejected JSON line for error correlation.
pub fn extract_best_effort_id(raw_line: &str) -> u64 {
    if raw_line.len() > MAX_DEV_COMMAND_BYTES {
        return 0;
    }
    serde_json::from_str::<serde_json::Value>(raw_line.trim())
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_u64()))
        .unwrap_or(0)
}

/// Read one `\n`-terminated line up to `max_bytes` without buffering unbounded data in memory.
/// Returns:
/// - `Ok(None)` on EOF with zero bytes read
/// - `Ok(Some(Ok(line)))` on valid line within `max_bytes`
/// - `Ok(Some(Err(byte_count)))` when the line exceeded `max_bytes` (drained up to `\n`/EOF)
pub fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
) -> std::io::Result<Option<Result<String, usize>>> {
    let mut buf = Vec::new();
    let mut total_bytes = 0usize;
    let mut exceeded = false;

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if total_bytes == 0 {
                return Ok(None);
            }
            break;
        }

        if let Some(newline_pos) = available.iter().position(|&b| b == b'\n') {
            let chunk_len = newline_pos + 1;
            total_bytes = total_bytes.saturating_add(chunk_len);
            if !exceeded && buf.len() + chunk_len <= max_bytes {
                buf.extend_from_slice(&available[..chunk_len]);
            } else {
                exceeded = true;
            }
            reader.consume(chunk_len);
            break;
        }

        let chunk_len = available.len();
        total_bytes = total_bytes.saturating_add(chunk_len);
        if !exceeded && buf.len() + chunk_len <= max_bytes {
            buf.extend_from_slice(available);
        } else {
            exceeded = true;
        }
        reader.consume(chunk_len);
    }

    if exceeded {
        return Ok(Some(Err(total_bytes)));
    }

    let line = String::from_utf8(buf).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("non-UTF-8 command line: {e}"),
        )
    })?;
    Ok(Some(Ok(line)))
}

/// Validate dev-shell-only startup preconditions:
/// 1. `--dev` flag must be set.
/// 2. Neither `asset_root` nor `project_root` may contain `wavical-manifest.json` (refuses built apps).
/// 3. `WAVICAL_DEV_SESSION_TOKEN` must be present and non-empty.
pub fn validate_dev_startup_requirements(
    dev_flag: bool,
    asset_root: &Path,
    project_root: Option<&Path>,
    env_token: Option<&str>,
) -> Result<String, String> {
    if !dev_flag {
        return Err(
            "wavical-shell-dev refuses to start without --dev (use wavical-shell for built apps)"
                .to_string(),
        );
    }

    if asset_root.join("wavical-manifest.json").is_file()
        || project_root.is_some_and(|p| p.join("wavical-manifest.json").is_file())
    {
        return Err(
            "wavical-shell-dev refuses to load a built application (wavical-manifest.json present); use wavical-shell for built apps"
                .to_string(),
        );
    }

    let Some(token) = env_token.map(str::trim).filter(|t| !t.is_empty()) else {
        return Err(format!(
            "wavical-shell-dev requires parent session token in {DEV_SESSION_TOKEN_ENV}"
        ));
    };

    Ok(token.to_string())
}

/// Write a single JSON reply line to stdout and flush immediately.
pub fn write_reply(reply: &DevCommandReply) -> std::io::Result<()> {
    let json = serde_json::to_string(reply)
        .map_err(|e| std::io::Error::other(format!("serialize reply: {e}")))?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{json}")?;
    stdout.flush()
}

/// Runtime command dispatched from the stdin channel to the tao UI event loop.
#[derive(Debug)]
pub enum DevLoopAction {
    Reload,
    ReloadConfig {
        capabilities: Vec<String>,
        network_mode: Option<String>,
        remote_url: Option<String>,
        dev_url: Option<String>,
    },
    RestartBackend,
    Exit,
}

/// Envelope delivered to the tao UI thread with a one-shot acknowledgement channel.
#[derive(Debug)]
pub struct DevLoopRequest {
    pub action: DevLoopAction,
    pub ack_tx: mpsc::SyncSender<Result<(), String>>,
}

/// Handle returned by [`start_dev_stdin_session`] once the initial `auth` handshake succeeds.
pub struct DevSessionHandle {
    pub marker: &'static str,
    proxy_tx: mpsc::SyncSender<crate::UserEventProxyHandle>,
    auth_id: u64,
    auth_started: Instant,
}

impl DevSessionHandle {
    /// Attach the tao event loop proxy so the stdin thread can write the `auth` reply
    /// and begin dispatching live dev commands.
    pub fn activate(self, proxy: tao::event_loop::EventLoopProxy<crate::UserEvent>) {
        let elapsed_ms = self.auth_started.elapsed().as_millis() as u64;
        let _ = write_reply(&DevCommandReply::ok(self.auth_id, elapsed_ms));
        let _ = self.proxy_tx.send(crate::UserEventProxyHandle(proxy));
    }
}

/// Authenticate the stdin command channel BEFORE opening any window, then keep the
/// single stdin reader thread alive for the lifetime of the dev shell session.
pub fn start_dev_stdin_session(expected_token: String) -> Result<DevSessionHandle, String> {
    let (auth_tx, auth_rx) = mpsc::sync_channel::<Result<(u64, Instant), String>>(1);
    let (proxy_tx, proxy_rx) = mpsc::sync_channel::<crate::UserEventProxyHandle>(1);

    thread::Builder::new()
        .name("wavical-dev-stdin".to_string())
        .spawn(move || {
            let stdin: Stdin = std::io::stdin();
            let mut reader = BufReader::new(stdin.lock());

            // Step 1: Wait for initial `auth` command with matching session token
            let first_line = match read_bounded_line(&mut reader, MAX_DEV_COMMAND_BYTES) {
                Ok(Some(Ok(line))) => line,
                Ok(Some(Err(sz))) => {
                    let msg = format!(
                        "initial auth command exceeds maximum size of {MAX_DEV_COMMAND_BYTES} bytes (got {sz})"
                    );
                    let _ = write_reply(&DevCommandReply::err(0, &msg, 0));
                    let _ = auth_tx.send(Err(msg));
                    return;
                }
                Ok(None) => {
                    let _ = auth_tx.send(Err(
                        "stdin closed before parent session token handshake".to_string(),
                    ));
                    return;
                }
                Err(e) => {
                    let _ = auth_tx.send(Err(format!("failed reading stdin handshake: {e}")));
                    return;
                }
            };

            let auth_started = Instant::now();
            let first_cmd = match parse_dev_command(&first_line) {
                Ok(cmd) => cmd,
                Err(e) => {
                    let id = extract_best_effort_id(&first_line);
                    let msg = e.to_string();
                    let _ = write_reply(&DevCommandReply::err(id, &msg, 0));
                    let _ = auth_tx.send(Err(msg));
                    return;
                }
            };

            let DevCommand::Auth { id, token } = first_cmd else {
                let id = first_cmd.id();
                let msg = "first command on dev channel must be 'auth' with session token"
                    .to_string();
                let _ = write_reply(&DevCommandReply::err(id, &msg, 0));
                let _ = auth_tx.send(Err(msg));
                return;
            };

            if !constant_time_eq(token.as_bytes(), expected_token.as_bytes()) {
                let msg = "invalid dev session token".to_string();
                let _ = write_reply(&DevCommandReply::err(id, &msg, 0));
                let _ = auth_tx.send(Err(msg));
                return;
            }

            if auth_tx.send(Ok((id, auth_started))).is_err() {
                return;
            }

            // Wait for main thread to finish window/event loop initialization and give us proxy
            let Ok(proxy_handle) = proxy_rx.recv() else {
                return;
            };
            let proxy = proxy_handle.0;

            // Step 2: Command loop
            loop {
                match read_bounded_line(&mut reader, MAX_DEV_COMMAND_BYTES) {
                    Ok(Some(Ok(line))) => {
                        if line.trim().is_empty() {
                            continue;
                        }
                        let cmd_start = Instant::now();
                        let cmd = match parse_dev_command(&line) {
                            Ok(c) => c,
                            Err(e) => {
                                let id = extract_best_effort_id(&line);
                                let _ = write_reply(&DevCommandReply::err(
                                    id,
                                    e.to_string(),
                                    cmd_start.elapsed().as_millis() as u64,
                                ));
                                continue;
                            }
                        };

                        let id = cmd.id();
                        if !constant_time_eq(cmd.token().as_bytes(), expected_token.as_bytes()) {
                            let _ = write_reply(&DevCommandReply::err(
                                id,
                                "invalid dev session token",
                                cmd_start.elapsed().as_millis() as u64,
                            ));
                            continue;
                        }

                        match cmd {
                            DevCommand::Auth { .. } => {
                                let _ = write_reply(&DevCommandReply::ok(
                                    id,
                                    cmd_start.elapsed().as_millis() as u64,
                                ));
                            }
                            DevCommand::Reload { .. } => {
                                dispatch_and_reply(&proxy, id, DevLoopAction::Reload, cmd_start);
                            }
                            DevCommand::ReloadConfig {
                                capabilities,
                                network_mode,
                                remote_url,
                                dev_url,
                                ..
                            } => {
                                dispatch_and_reply(
                                    &proxy,
                                    id,
                                    DevLoopAction::ReloadConfig {
                                        capabilities,
                                        network_mode,
                                        remote_url,
                                        dev_url,
                                    },
                                    cmd_start,
                                );
                            }
                            DevCommand::RestartBackend { .. } => {
                                dispatch_and_reply(
                                    &proxy,
                                    id,
                                    DevLoopAction::RestartBackend,
                                    cmd_start,
                                );
                            }
                            DevCommand::Exit { .. } => {
                                dispatch_and_reply(&proxy, id, DevLoopAction::Exit, cmd_start);
                                break;
                            }
                        }
                    }
                    Ok(Some(Err(sz))) => {
                        let _ = write_reply(&DevCommandReply::err(
                            0,
                            format!(
                                "dev command exceeds maximum size of {MAX_DEV_COMMAND_BYTES} bytes (got {sz} bytes)"
                            ),
                            0,
                        ));
                    }
                    Ok(None) | Err(_) => {
                        // Parent closed stdin -> exit shell cleanly
                        let (ack_tx, _ack_rx) = mpsc::sync_channel(1);
                        let _ = proxy.send_event(crate::UserEvent::DevCommand(DevLoopRequest {
                            action: DevLoopAction::Exit,
                            ack_tx,
                        }));
                        break;
                    }
                }
            }
        })
        .map_err(|e| format!("failed to spawn dev stdin thread: {e}"))?;

    match auth_rx.recv_timeout(DEV_AUTH_HANDSHAKE_TIMEOUT) {
        Ok(Ok((auth_id, auth_started))) => Ok(DevSessionHandle {
            marker: DEV_SHELL_BINARY_MARKER,
            proxy_tx,
            auth_id,
            auth_started,
        }),
        Ok(Err(reason)) => Err(reason),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(
            "timed out waiting for parent session token handshake on command channel".to_string(),
        ),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("stdin command channel closed during handshake".to_string())
        }
    }
}

fn dispatch_and_reply(
    proxy: &tao::event_loop::EventLoopProxy<crate::UserEvent>,
    id: u64,
    action: DevLoopAction,
    cmd_start: Instant,
) {
    let (ack_tx, ack_rx) = mpsc::sync_channel(1);
    if proxy
        .send_event(crate::UserEvent::DevCommand(DevLoopRequest {
            action,
            ack_tx,
        }))
        .is_err()
    {
        let _ = write_reply(&DevCommandReply::err(
            id,
            "shell event loop is no longer running",
            cmd_start.elapsed().as_millis() as u64,
        ));
        return;
    }

    match ack_rx.recv_timeout(Duration::from_secs(4)) {
        Ok(Ok(())) => {
            let _ = write_reply(&DevCommandReply::ok(
                id,
                cmd_start.elapsed().as_millis() as u64,
            ));
        }
        Ok(Err(err)) => {
            let _ = write_reply(&DevCommandReply::err(
                id,
                err,
                cmd_start.elapsed().as_millis() as u64,
            ));
        }
        Err(_) => {
            let _ = write_reply(&DevCommandReply::err(
                id,
                "shell UI thread did not acknowledge command in time",
                cmd_start.elapsed().as_millis() as u64,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_parse_valid_commands() {
        assert_eq!(
            parse_dev_command(r#"{"cmd":"auth","id":0,"token":"tok123"}"#).unwrap(),
            DevCommand::Auth {
                id: 0,
                token: "tok123".to_string()
            }
        );
        assert_eq!(
            parse_dev_command(r#"{"cmd":"reload","id":1,"token":"tok123"}"#).unwrap(),
            DevCommand::Reload {
                id: 1,
                token: "tok123".to_string()
            }
        );
        assert_eq!(
            parse_dev_command(
                r#"{"cmd":"reload-config","id":2,"token":"tok123","capabilities":["core.ping"],"networkMode":"safe","remoteUrl":"https://app.example.com","devUrl":"http://localhost:5173"}"#
            )
            .unwrap(),
            DevCommand::ReloadConfig {
                id: 2,
                token: "tok123".to_string(),
                capabilities: vec!["core.ping".to_string()],
                network_mode: Some("safe".to_string()),
                remote_url: Some("https://app.example.com".to_string()),
                dev_url: Some("http://localhost:5173".to_string()),
            }
        );
        assert_eq!(
            parse_dev_command(r#"{"cmd":"restart-backend","id":3,"token":"tok123"}"#).unwrap(),
            DevCommand::RestartBackend {
                id: 3,
                token: "tok123".to_string()
            }
        );
        assert_eq!(
            parse_dev_command(r#"{"cmd":"exit","id":4,"token":"tok123"}"#).unwrap(),
            DevCommand::Exit {
                id: 4,
                token: "tok123".to_string()
            }
        );
    }

    #[test]
    fn test_parse_rejects_malformed_oversize_unknown_and_extra_fields() {
        // Malformed JSON / empty
        assert!(matches!(
            parse_dev_command(""),
            Err(DevCommandParseError::Malformed(_))
        ));
        assert!(matches!(
            parse_dev_command("{not json"),
            Err(DevCommandParseError::Malformed(_))
        ));

        // Unknown command
        assert!(matches!(
            parse_dev_command(r#"{"cmd":"eval-js","id":1,"token":"tok"}"#),
            Err(DevCommandParseError::Invalid(_))
        ));

        // Extra unknown field rejected by deny_unknown_fields
        assert!(matches!(
            parse_dev_command(r#"{"cmd":"reload","id":1,"token":"tok","extra":true}"#),
            Err(DevCommandParseError::Invalid(_))
        ));

        // Missing or empty token rejected
        assert!(parse_dev_command(r#"{"cmd":"reload","id":1}"#).is_err());
        assert!(matches!(
            parse_dev_command(r#"{"cmd":"reload","id":1,"token":"   "}"#),
            Err(DevCommandParseError::Invalid(_))
        ));

        // Oversize command rejected
        let huge = format!(
            r#"{{"cmd":"reload","id":1,"token":"{}"}}"#,
            "a".repeat(MAX_DEV_COMMAND_BYTES)
        );
        assert!(matches!(
            parse_dev_command(&huge),
            Err(DevCommandParseError::TooLarge { .. })
        ));
    }

    #[test]
    fn test_read_bounded_line_rejects_oversize_without_oom() {
        let huge_line = format!("{}\n{{\"cmd\":\"reload\"}}\n", "x".repeat(200));
        let mut cursor = Cursor::new(huge_line.as_bytes());

        let first = read_bounded_line(&mut cursor, 64).unwrap().unwrap();
        assert!(matches!(first, Err(len) if len > 200));

        let second = read_bounded_line(&mut cursor, 64).unwrap().unwrap();
        assert_eq!(second.unwrap(), "{\"cmd\":\"reload\"}\n");

        assert!(read_bounded_line(&mut cursor, 64).unwrap().is_none());
    }

    #[test]
    fn test_validate_dev_startup_requirements() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        // Missing --dev rejected
        assert!(validate_dev_startup_requirements(false, &src, None, Some("tok")).is_err());
        // Missing token rejected
        assert!(validate_dev_startup_requirements(true, &src, None, None).is_err());
        assert!(validate_dev_startup_requirements(true, &src, None, Some("  ")).is_err());
        // Valid dev dir succeeds
        assert_eq!(
            validate_dev_startup_requirements(true, &src, Some(tmp.path()), Some("tok123"))
                .unwrap(),
            "tok123"
        );

        // Built manifest in asset_root or project_root rejected
        let dist = tmp.path().join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("wavical-manifest.json"), "{}").unwrap();
        assert!(validate_dev_startup_requirements(true, &dist, None, Some("tok123")).is_err());
        assert!(
            validate_dev_startup_requirements(true, &src, Some(&dist), Some("tok123")).is_err()
        );
    }
}
