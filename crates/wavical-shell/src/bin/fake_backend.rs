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
