#![forbid(unsafe_code)]

//! Test-only HTTP probe server for built apps (`#[cfg(feature = "test-probe")]`).
//! Never compiled into the production `wavical-shell` binary.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tao::event_loop::EventLoopProxy;

use crate::UserEvent;

/// Shared one-shot reply slot for test-probe JavaScript evaluations.
pub type ProbeReplyTx = Arc<Mutex<Option<mpsc::Sender<Result<String, String>>>>>;

/// Unique binary marker present only in `wavical-shell-test-probe` (`#[cfg(feature = "test-probe")]`).
/// Checked negatively against production `wavical-shell.exe` by package verification.
pub const TEST_PROBE_BINARY_MARKER: &str = "WAVICAL_TEST_PROBE_V1_MARKER";

const MAX_PROBE_BODY_BYTES: usize = 256 * 1024;
const EVAL_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalRequest {
    script: String,
}

#[derive(Debug, Serialize)]
struct EvalResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Starts a loopback-only HTTP probe server on `127.0.0.1:0`, writes the bound port to
/// `port_file`, and forwards `POST /eval` scripts to the WebView UI thread.
pub fn start_probe_server(
    proxy: EventLoopProxy<UserEvent>,
    port_file: &Path,
) -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("failed to bind test probe listener: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("failed to inspect test probe listener address: {e}"))?
        .port();

    if let Some(parent) = port_file.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create probe port parent dir: {e}"))?;
    }
    let tmp_path = port_file.with_extension("tmp");
    fs::write(&tmp_path, port.to_string())
        .map_err(|e| format!("failed to write temp probe port file: {e}"))?;
    fs::rename(&tmp_path, port_file)
        .map_err(|e| format!("failed to finalize probe port file: {e}"))?;

    thread::spawn(move || {
        for stream_res in listener.incoming() {
            let Ok(mut stream) = stream_res else {
                break;
            };
            let proxy_c = proxy.clone();
            thread::spawn(move || {
                let _ = handle_connection(&mut stream, &proxy_c);
            });
        }
    });

    Ok(port)
}

fn handle_connection(
    stream: &mut std::net::TcpStream,
    proxy: &EventLoopProxy<UserEvent>,
) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    let mut content_length = 0usize;
    loop {
        let mut header_line = String::new();
        if reader.read_line(&mut header_line)? == 0 {
            break;
        }
        let trimmed = header_line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
            && let Ok(n) = v.trim().parse::<usize>()
        {
            content_length = n;
        }
    }

    if content_length > MAX_PROBE_BODY_BYTES {
        return write_json(
            stream,
            413,
            &EvalResponse {
                ok: false,
                result: None,
                error: Some("probe payload too large".to_string()),
            },
        );
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    match (method, path) {
        ("GET", "/health") => write_json(
            stream,
            200,
            &EvalResponse {
                ok: true,
                result: Some(TEST_PROBE_BINARY_MARKER.to_string()),
                error: None,
            },
        ),
        ("POST", "/eval") => {
            let req: EvalRequest = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    return write_json(
                        stream,
                        400,
                        &EvalResponse {
                            ok: false,
                            result: None,
                            error: Some(format!("invalid probe JSON: {e}")),
                        },
                    );
                }
            };
            let (tx, rx) = mpsc::channel();
            let slot = Arc::new(Mutex::new(Some(tx)));
            if let Err(e) = proxy.send_event(UserEvent::EvalProbe {
                script: req.script,
                reply_tx: slot,
            }) {
                return write_json(
                    stream,
                    500,
                    &EvalResponse {
                        ok: false,
                        result: None,
                        error: Some(format!("event loop unavailable: {e}")),
                    },
                );
            }
            match rx.recv_timeout(EVAL_TIMEOUT) {
                Ok(Ok(val)) => write_json(
                    stream,
                    200,
                    &EvalResponse {
                        ok: true,
                        result: Some(val),
                        error: None,
                    },
                ),
                Ok(Err(err)) => write_json(
                    stream,
                    500,
                    &EvalResponse {
                        ok: false,
                        result: None,
                        error: Some(err),
                    },
                ),
                Err(_) => write_json(
                    stream,
                    504,
                    &EvalResponse {
                        ok: false,
                        result: None,
                        error: Some("probe evaluation timed out".to_string()),
                    },
                ),
            }
        }
        _ => write_json(
            stream,
            404,
            &EvalResponse {
                ok: false,
                result: None,
                error: Some("not found".to_string()),
            },
        ),
    }
}

fn write_json(
    stream: &mut std::net::TcpStream,
    status: u16,
    payload: &EvalResponse,
) -> std::io::Result<()> {
    let body = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Payload Too Large",
        504 => "Gateway Timeout",
        _ => "Internal Server Error",
    };
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(resp.as_bytes())?;
    stream.flush()
}
