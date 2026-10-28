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
