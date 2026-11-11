#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::env;
use std::io::{Read, Write};
use std::thread;
use std::time::Duration;

use interprocess::local_socket::traits::Stream as StreamTrait;
use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct BackendEnvironmentReport {
    pub args: Vec<String>,
    pub env_vars: BTreeMap<String, String>,
    pub cwd: String,
}

fn main() {
    let args: Vec<String> = env::args().collect();

    // 1. Crash on request to test crash limit / restart policy
    if args.iter().any(|a| a == "--crash") {
        eprintln!("fake_backend: intentional crash requested via --crash");
        std::process::exit(1);
    }

    // 2. Emit some stdout and stderr lines for log reader testing
    println!("fake_backend starting: pid={}", std::process::id());
    eprintln!("fake_backend stderr initial log line");

    if args.iter().any(|a| a == "--test-log-caps") {
        // Emit a long line (> 4 KB) to verify truncation
        let long_line = "A".repeat(5000);
        println!("LONG:{}", long_line);

        // Emit rapid lines to verify rate limiting
        for i in 0..250 {
            println!("RAPID_LOG_{}", i);
        }
    }

    // 3. Connect to the Wavical IPC pipe if provided
    if let Ok(pipe_name) = env::var("WAVICAL_IPC_PIPE") {
        let ns_name = match pipe_name.clone().to_ns_name::<GenericNamespaced>() {
            Ok(name) => name,
            Err(e) => {
                eprintln!("fake_backend: failed to construct pipe ns_name: {e}");
                std::process::exit(2);
            }
        };

        // Retry connecting for up to 2 seconds if listener is busy
        let mut stream_opt = None;
        for _ in 0..20 {
            if let Ok(stream) = Stream::connect(ns_name.clone()) {
                stream_opt = Some(stream);
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        if let Some(mut stream) = stream_opt {
            if args.iter().any(|a| a == "--protocol") {
                run_protocol_loop(&mut stream);
                return;
            }

            let report = BackendEnvironmentReport {
                args: args.clone(),
                env_vars: env::vars().collect(),
                cwd: env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            };

            let payload = serde_json::to_vec(&report).expect("failed to serialize report");
            let len_bytes = (payload.len() as u32).to_le_bytes();

            if let Err(e) = stream.write_all(&len_bytes) {
                eprintln!("fake_backend: failed to write length prefix: {e}");
            }
            if let Err(e) = stream.write_all(&payload) {
                eprintln!("fake_backend: failed to write payload: {e}");
            }
            let _ = stream.flush();

            // If exit immediately flag is present, quit now
            if args.iter().any(|a| a == "--exit-immediately") {
                return;
            }

            // Keep connection alive until closed by shell
            let mut dummy = [0u8; 1];
            let _ = stream.read(&mut dummy);
        } else {
            eprintln!("fake_backend: could not connect to pipe {pipe_name}");
        }
    }

    if args.iter().any(|a| a == "--sleep") {
        thread::sleep(Duration::from_secs(30));
    }
}

fn run_protocol_loop(stream: &mut Stream) {
    let mut buf = wavical_ipc::BackendFrameBuffer::new();
    let mut chunk = [0u8; 8192];

    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buf.push_bytes(&chunk[..n]).is_err() {
                    break;
                }

                while let Ok(Some(req)) = buf.pop_request_frame() {
                    handle_fake_protocol_request(stream, req);
                }
            }
            Err(_) => break,
        }
    }
}

fn handle_fake_protocol_request(stream: &mut Stream, req: wavical_ipc::BackendRequestFrame) {
    match req.method.as_str() {
        "hello.greet" => {
            let name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("World");
            let resp = wavical_ipc::BackendResponseFrame {
                id: req.id,
                ok: true,
                result: Some(serde_json::json!({ "message": format!("Hello, {}!", name) })),
                error: None,
            };
            let bytes = wavical_ipc::encode_response_frame(&resp).expect("encode");
            let _ = stream.write_all(&bytes);
            let _ = stream.flush();
        }
        "test.echo" => {
            let resp = wavical_ipc::BackendResponseFrame {
                id: req.id,
                ok: true,
                result: Some(req.params),
                error: None,
            };
            let bytes = wavical_ipc::encode_response_frame(&resp).expect("encode");
            let _ = stream.write_all(&bytes);
            let _ = stream.flush();
        }
        "test.sleep" => {
            // Slow method: do not reply immediately
            thread::spawn(|| {
                thread::sleep(Duration::from_secs(10));
            });
        }
        "test.answer_twice" => {
            let resp1 = wavical_ipc::BackendResponseFrame {
                id: req.id,
                ok: true,
                result: Some(serde_json::json!({ "call": 1 })),
                error: None,
            };
            let bytes1 = wavical_ipc::encode_response_frame(&resp1).expect("encode");
            let _ = stream.write_all(&bytes1);
            let _ = stream.flush();

            let resp2 = wavical_ipc::BackendResponseFrame {
                id: req.id,
                ok: true,
                result: Some(serde_json::json!({ "call": 2 })),
                error: None,
            };
            let bytes2 = wavical_ipc::encode_response_frame(&resp2).expect("encode");
            let _ = stream.write_all(&bytes2);
            let _ = stream.flush();
        }
        "test.unknown_id" => {
            let resp = wavical_ipc::BackendResponseFrame {
                id: 999_999,
                ok: true,
                result: Some(serde_json::json!({ "unknown": true })),
                error: None,
            };
            let bytes = wavical_ipc::encode_response_frame(&resp).expect("encode");
            let _ = stream.write_all(&bytes);
            let _ = stream.flush();
        }
        "test.interleave" => {
            let delay_ms = req
                .params
                .get("delay_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if delay_ms > 0 {
                thread::sleep(Duration::from_millis(delay_ms));
            }
            let resp = wavical_ipc::BackendResponseFrame {
                id: req.id,
                ok: true,
                result: Some(serde_json::json!({ "delay_ms": delay_ms })),
                error: None,
            };
            let bytes = wavical_ipc::encode_response_frame(&resp).expect("encode");
            let _ = stream.write_all(&bytes);
            let _ = stream.flush();
        }
        "test.heavy_logging" => {
            for i in 0..50 {
                println!("STDOUT_LOG_LINE_{}: {}", i, "x".repeat(3000));
                eprintln!("STDERR_LOG_LINE_{}: {}", i, "y".repeat(3000));
            }
            let resp = wavical_ipc::BackendResponseFrame {
                id: req.id,
                ok: true,
                result: Some(serde_json::json!({ "logged": true })),
                error: None,
            };
            let bytes = wavical_ipc::encode_response_frame(&resp).expect("encode");
            let _ = stream.write_all(&bytes);
            let _ = stream.flush();
        }
        "test.malformed_json" => {
            let bad_json = b"{\"id\":1, not valid json}";
            let len = (bad_json.len() as u32).to_be_bytes();
            let _ = stream.write_all(&len);
            let _ = stream.write_all(bad_json);
            let _ = stream.flush();
        }
        "test.oversize_frame" => {
            let fake_len = (wavical_ipc::MAX_BACKEND_RESPONSE_BYTES as u32) + 1024;
            let _ = stream.write_all(&fake_len.to_be_bytes());
            let _ = stream.flush();
        }
        "test.error" => {
            let resp = wavical_ipc::BackendResponseFrame {
                id: req.id,
                ok: false,
                result: None,
                error: Some(
                    "Internal database connection error at /var/secret/db.sqlite".to_string(),
                ),
            };
            let bytes = wavical_ipc::encode_response_frame(&resp).expect("encode");
            let _ = stream.write_all(&bytes);
            let _ = stream.flush();
        }
        _ => {
            let resp = wavical_ipc::BackendResponseFrame {
                id: req.id,
                ok: false,
                result: None,
                error: Some(format!("Unknown fake_backend method '{}'", req.method)),
            };
            let bytes = wavical_ipc::encode_response_frame(&resp).expect("encode");
            let _ = stream.write_all(&bytes);
            let _ = stream.flush();
        }
    }
}
