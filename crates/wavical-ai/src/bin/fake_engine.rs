#![forbid(unsafe_code)]

//! A tiny mock engine executable speaking the same loopback HTTP protocol as `llama-server`.
//!
//! Used for deterministic automated testing of engine process management, health handshakes,
//! environment hygiene, command-line hygiene, and kill-on-parent-close Job Object behavior.

use std::env;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

fn main() {
    let args: Vec<String> = env::args().collect();

    // Check for crash test trigger
    if args.iter().any(|a| a == "--crash-immediately") {
        std::process::exit(101);
    }

    let mut host = "127.0.0.1".to_string();
    let mut port = 0u16;
    let mut api_key = String::new();
    let mut fail_health = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--host" if i + 1 < args.len() => {
                host = args[i + 1].clone();
                i += 2;
            }
            "--port" if i + 1 < args.len() => {
                port = args[i + 1].parse().unwrap_or(0);
                i += 2;
            }
            "--api-key" if i + 1 < args.len() => {
                api_key = args[i + 1].clone();
                i += 2;
            }
            "--fail-health" => {
                fail_health = true;
                i += 1;
            }
            _ => i += 1,
        }
    }

    let addr = format!("{host}:{port}");
    let listener = TcpListener::bind(&addr).expect("fake engine bind loopback port");

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);
    let expected_key = api_key.clone();
    let recorded_args = args.clone();

    // Check environment hygiene: detect any leaked secrets
    let has_forbidden_secret = env::vars()
        .any(|(k, _)| k.contains("SECRET") || k.contains("API_KEY") || k.contains("PRIVATE"));

    thread::spawn(move || {
        while running_clone.load(Ordering::SeqCst) {
            if let Ok((mut stream, _)) = listener.accept() {
                handle_client(
                    &mut stream,
                    &expected_key,
                    fail_health,
                    has_forbidden_secret,
                    &recorded_args,
                );
            }
        }
    });

    // Keep running until killed
    loop {
        thread::sleep(std::time::Duration::from_millis(500));
    }
}

fn handle_client(
    stream: &mut TcpStream,
    expected_key: &str,
    fail_health: bool,
    has_forbidden_secret: bool,
    recorded_args: &[String],
) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut req_line = String::new();
    if reader.read_line(&mut req_line).is_err() || req_line.is_empty() {
        return;
    }

    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
            break;
        }
        headers.push(line.trim().to_string());
    }

    // Check authorization header
    let expected_auth = format!("Authorization: Bearer {}", expected_key);
    let is_authorized = headers
        .iter()
        .any(|h| h.eq_ignore_ascii_case(&expected_auth));

    if !is_authorized && !expected_key.is_empty() {
        let resp = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.flush();
        return;
    }

    let content_length: usize = headers
        .iter()
        .find_map(|h| {
            if h.to_ascii_lowercase().starts_with("content-length:") {
                h.split(':').nth(1)?.trim().parse().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    let mut body_bytes = vec![0u8; content_length];
    if content_length > 0 {
        use std::io::Read;
        let _ = reader.read_exact(&mut body_bytes);
    }
    let body_str = String::from_utf8_lossy(&body_bytes);

    let path = req_line.split_whitespace().nth(1).unwrap_or("/");

    if path == "/health" {
        if fail_health {
            let resp = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes());
        } else {
            let body = r#"{"status":"ok"}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    } else if path == "/test-env" {
        let env_keys: Vec<String> = env::vars().map(|(k, _)| k).collect();
        let body = serde_json::json!({
            "has_forbidden_secret": has_forbidden_secret,
            "env_keys": env_keys,
            "args": recorded_args
        })
        .to_string();
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(resp.as_bytes());
    } else if path == "/v1/chat/completions" {
        if body_str.contains("test:crash_mid_stream") {
            let sse_header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(sse_header.as_bytes());
            let chunk = "data: {\"choices\":[{\"delta\":{\"content\":\"initial chunk before crash\"}}]}\n\n";
            let _ = stream.write_all(chunk.as_bytes());
            let _ = stream.flush();
            std::thread::sleep(std::time::Duration::from_millis(50));
            // Simulate sudden process crash mid-stream
            std::process::exit(99);
        } else if body_str.contains("test:abrupt_disconnect") {
            let sse_header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(sse_header.as_bytes());
            let chunk =
                "data: {\"choices\":[{\"delta\":{\"content\":\"first and only chunk\"}}]}\n\n";
            let _ = stream.write_all(chunk.as_bytes());
            let _ = stream.flush();
            // Drop connection without sending [DONE]
            return;
        } else if body_str.contains("test:hang_timeout") {
            let sse_header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(sse_header.as_bytes());
            let _ = stream.flush();
            // Hang for 130 seconds to trigger the 120s wall-clock timeout
            std::thread::sleep(std::time::Duration::from_secs(130));
            return;
        } else if body_str.contains("test:xss_markup") {
            let sse_header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(sse_header.as_bytes());
            let chunks = [
                "data: {\"choices\":[{\"delta\":{\"content\":\"</script><script>alert(1)</script>\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"<img src=x onerror=alert('xss')>\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"Line 1\u{2028}Line 2\u{2029}Line 3\"}}]}\n\n",
                "data: [DONE]\n\n",
            ];
            for c in chunks {
                let _ = stream.write_all(c.as_bytes());
                let _ = stream.flush();
            }
        } else if body_str.contains("test:oversized_5mb") {
            let sse_header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(sse_header.as_bytes());
            // Send 5 MB across 80 chunks of 64 KB
            let block = "A".repeat(64 * 1024);
            let chunk_json = serde_json::json!({
                "choices": [{
                    "delta": {
                        "content": block
                    }
                }]
            })
            .to_string();
            let chunk_sse = format!("data: {chunk_json}\n\n");
            for _ in 0..80 {
                let _ = stream.write_all(chunk_sse.as_bytes());
                let _ = stream.flush();
            }
            let _ = stream.write_all(b"data: [DONE]\n\n");
            let _ = stream.flush();
        } else if body_str.contains("test:invalid_utf8") {
            let sse_header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(sse_header.as_bytes());
            // Raw non-UTF-8 bytes inside SSE payload
            let mut raw_chunk = Vec::new();
            raw_chunk
                .extend_from_slice(b"data: {\"choices\":[{\"delta\":{\"content\":\"valid prefix ");
            raw_chunk.extend_from_slice(&[0xFF, 0xFE, 0xFD]); // Invalid UTF-8 sequence
            raw_chunk.extend_from_slice(b" valid suffix\"}}]}\n\n");
            let _ = stream.write_all(&raw_chunk);
            let _ = stream.flush();
            let _ = stream.write_all(b"data: [DONE]\n\n");
            let _ = stream.flush();
        } else if body_str.contains("test:slow_stream") {
            let sse_header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(sse_header.as_bytes());
            for i in 1..=10 {
                let chunk = format!(
                    "data: {{\"choices\":[{{\"delta\":{{\"content\":\"token{i} \"}}}}]}}\n\n"
                );
                let _ = stream.write_all(chunk.as_bytes());
                let _ = stream.flush();
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
            let _ = stream.write_all(b"data: [DONE]\n\n");
            let _ = stream.flush();
        } else {
            // Standard normal completion
            let sse_header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(sse_header.as_bytes());
            let chunks = [
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hello \"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"from fake engine!\"}}]}\n\n",
                "data: [DONE]\n\n",
            ];
            for c in chunks {
                let _ = stream.write_all(c.as_bytes());
                let _ = stream.flush();
            }
        }
    } else if path == "/v1/embeddings" {
        if body_str.contains("test:embedding_failure") {
            let resp = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
            return;
        }

        if body_str.contains("test:embedding_crash") {
            std::process::exit(42);
        }

        let dim = if body_str.contains("nomic") { 768 } else { 384 };

        // Parse input: either a single string or an array of strings
        let parsed: serde_json::Value = serde_json::from_str(&body_str).unwrap_or_default();
        let inputs: Vec<String> = if let Some(arr) = parsed.get("input").and_then(|v| v.as_array())
        {
            arr.iter()
                .filter_map(|s| s.as_str().map(|t| t.to_string()))
                .collect()
        } else if let Some(s) = parsed.get("input").and_then(|v| v.as_str()) {
            vec![s.to_string()]
        } else {
            vec!["default".to_string()]
        };

        let data: Vec<serde_json::Value> = inputs
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let mut vec = vec![0.0f32; dim];
                for (char_idx, b) in text.as_bytes().iter().enumerate() {
                    vec[char_idx % dim] += (*b as f32) / 255.0;
                    vec[(char_idx * 7 + 3) % dim] += ((*b as f32) * 1.5) / 255.0;
                }
                let norm = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
                let norm = if norm > 0.0 { norm } else { 1.0 };
                for val in vec.iter_mut() {
                    *val /= norm;
                }

                serde_json::json!({
                    "object": "embedding",
                    "index": i,
                    "embedding": vec
                })
            })
            .collect();

        let resp_json = serde_json::json!({
            "object": "list",
            "data": data,
            "model": parsed.get("model").and_then(|m| m.as_str()).unwrap_or("bge-small-en-v1.5")
        });

        let body = resp_json.to_string();
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(resp.as_bytes());
    } else {
        let body = r#"{"status":"ok"}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(resp.as_bytes());
    }
    let _ = stream.flush();
}
