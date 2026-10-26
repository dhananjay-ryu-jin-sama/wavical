#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tao::{
    dpi::LogicalSize,
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder},
    window::{Window, WindowBuilder},
};
use wavical_shell::{
    IpcOriginError, WAVICAL_INIT_SCRIPT, WindowConfig, build_delivery_script,
    check_webview2_runtime, handle_navigation_request, handle_new_window_request,
    handle_protocol_request, process_ipc_message,
};
use wry::WebViewBuilder;

/// Structured attack result item emitted by tests/attack-page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttackTestItem {
    pub id: String,
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// Aggregated report from the attack page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttackReportSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttackReport {
    pub completed: bool,
    pub all_passed: bool,
    pub summary: AttackReportSummary,
    pub details: Vec<AttackTestItem>,
}

/// Custom events routed through the tao event loop.
#[derive(Debug, Clone)]
pub enum IntegrationEvent {
    ExecuteScript(String),
    ReportReceived(AttackReport),
    PollResults,
    Timeout,
}

fn locate_attack_page_asset_root() -> PathBuf {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let candidates = [
        current_dir.join("tests").join("attack-page"),
        current_dir
            .join("..")
            .join("..")
            .join("tests")
            .join("attack-page"),
    ];

    for candidate in &candidates {
        if candidate.join("index.html").is_file() {
            return candidate.clone();
        }
    }

    PathBuf::from("tests/attack-page")
}

fn main() {
    tracing_subscriber::fmt::init();
    println!("\n====================================================================");
    println!("[Wavical WP8 Integration Test Bench] Initializing Attack Test Runner");
    println!("====================================================================");

    // 1. Verify WebView2 runtime availability
    let webview_version = match check_webview2_runtime() {
        Ok(v) => v,
        Err(err) => {
            eprintln!("[Integration Test] SKIPPED: WebView2 runtime is not available: {err}");
            eprintln!("To run this integration test, install Microsoft Edge WebView2 Runtime.");
            // Return exit code 0 when runtime is missing in non-GUI / headless CI
            process::exit(0);
        }
    };
    println!("Detected WebView2 Runtime: {webview_version}");

    // 2. Configure window and asset root pointing to tests/attack-page
    let asset_root = locate_attack_page_asset_root();
    println!("Mounted Attack Page Asset Root: {}", asset_root.display());

    let config = WindowConfig {
        window_id: "integration-attack-window".to_string(),
        ..Default::default()
    };
    let effective_csp = config.effective_csp().to_string();

    // 3. Build tao event loop and window
    let event_loop: EventLoop<IntegrationEvent> = EventLoopBuilder::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window: Window = match WindowBuilder::new()
        .with_title("Wavical Security Test Bench (Attack Page)")
        .with_inner_size(LogicalSize::new(960.0, 750.0))
        .build(&event_loop)
    {
        Ok(w) => w,
        Err(err) => {
            eprintln!("[Integration Test] Failed to create host window: {err}");
            process::exit(1);
        }
    };

    // 4. Initialize handler registry with core and native handlers
    let mut registry = wavical_ipc::HandlerRegistry::with_core_handlers();
    let native_service = std::sync::Arc::new(wavical_native::NativeService::new());
    registry.register_native(native_service, &config.window_id);

    // 5. Configure protocol and handlers
    let protocol_asset_root = asset_root.clone();
    let protocol_csp = effective_csp.clone();
    let protocol_window_id = config.window_id.clone();
    let ipc_config = config.clone();
    let ipc_registry = registry.clone();
    let ipc_proxy = proxy.clone();
    let nav_allowed_origins = config.allowed_origins.clone();
    let nav_window_id = config.window_id.clone();
    let popup_window_id = config.window_id.clone();

    let finished_flag = Arc::new(AtomicBool::new(false));
    let finished_flag_clone = finished_flag.clone();
    let report_proxy = proxy.clone();

    let webview = match WebViewBuilder::new()
        .with_devtools(true)
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

            // Intercept attack completion report delivered via core.ping params
            if let Ok(raw_json) = serde_json::from_str::<serde_json::Value>(body) {
                let report_opt = raw_json
                    .get("params")
                    .and_then(|p| p.get("attack_report"))
                    .and_then(|r| serde_json::from_value::<AttackReport>(r.clone()).ok());

                if let Some(report) = report_opt
                    && !finished_flag_clone.swap(true, Ordering::SeqCst)
                {
                    let _ = report_proxy.send_event(IntegrationEvent::ReportReceived(report));
                }
            }

            match process_ipc_message(uri, body, &ipc_config, &ipc_registry) {
                Ok(Some(response)) => match build_delivery_script(&response) {
                    Ok(script) => {
                        let _ = ipc_proxy.send_event(IntegrationEvent::ExecuteScript(script));
                    }
                    Err(err) => {
                        eprintln!("Delivery script build error: {err}");
                    }
                },
                Ok(None) => {}
                Err(IpcOriginError::DisallowedOrigin(_)) => {}
                Err(IpcOriginError::InvalidUri) => {}
            }
        })
        .with_url("wavical://localhost/index.html")
        .build(&window)
    {
        Ok(wv) => wv,
        Err(err) => {
            eprintln!("[Integration Test] Failed to initialize WebView2: {err}");
            process::exit(1);
        }
    };

    // 6. Spawn polling and timeout background thread
    let timer_proxy = proxy.clone();
    let timer_finished = finished_flag.clone();
    std::thread::spawn(move || {
        let start = Instant::now();
        let timeout = Duration::from_secs(12);

        while start.elapsed() < timeout {
            std::thread::sleep(Duration::from_millis(200));
            if timer_finished.load(Ordering::SeqCst) {
                return;
            }
            let _ = timer_proxy.send_event(IntegrationEvent::PollResults);
        }

        if !timer_finished.load(Ordering::SeqCst) {
            let _ = timer_proxy.send_event(IntegrationEvent::Timeout);
        }
    });

    // 7. Event loop runner
    let poll_proxy = proxy.clone();
    let poll_finished = finished_flag.clone();

    println!("[Integration Test] Running attack suites in WebView2...");

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(StartCause::Init) => {
                println!("[Integration Test] Event loop initialized.");
            }
            Event::UserEvent(IntegrationEvent::ExecuteScript(script)) => {
                let _ = webview.evaluate_script(&script);
            }
            Event::UserEvent(IntegrationEvent::PollResults) => {
                if !poll_finished.load(Ordering::SeqCst) {
                    let callback_proxy = poll_proxy.clone();
                    let callback_finished = poll_finished.clone();

                    let _ = webview.evaluate_script_with_callback(
                        "window.__attack_results__ && window.__attack_results__.completed ? JSON.stringify(window.__attack_results__) : 'null'",
                        move |res_str| {
                            if res_str.trim() != "\"null\"" && !res_str.is_empty() {
                                // Strip outer quotes if serialized as JSON string
                                let unescaped = if res_str.starts_with('"') && res_str.ends_with('"') {
                                    serde_json::from_str::<String>(&res_str).unwrap_or(res_str)
                                } else {
                                    res_str
                                };

                                if let Ok(report) = serde_json::from_str::<AttackReport>(&unescaped)
                                    && report.completed
                                    && !callback_finished.swap(true, Ordering::SeqCst)
                                {
                                    let _ = callback_proxy.send_event(IntegrationEvent::ReportReceived(report));
                                }
                            }
                        },
                    );
                }
            }
            Event::UserEvent(IntegrationEvent::ReportReceived(report)) => {
                println!("\n====================================================================");
                println!("[WAVICAL INTEGRATION TEST BENCH - ATTACK SUITE RESULTS]");
                println!("====================================================================");
                println!(
                    "Total Attack Suites: {} | Thwarted (Pass): {} | Breached (Fail): {}",
                    report.summary.total, report.summary.passed, report.summary.failed
                );
                println!("--------------------------------------------------------------------");

                for item in &report.details {
                    let status_tag = if item.passed { "[PASS]" } else { "[FAIL]" };
                    println!("{status_tag} {}", item.name);
                    println!("       Reason/Verification: {}", item.detail);
                }

                println!("====================================================================");

                if report.all_passed && report.summary.passed == report.summary.total && report.summary.failed == 0 {
                    println!("SUCCESS: All {} adversarial attack vectors were securely thwarted!", report.summary.total);
                    println!("====================================================================\n");
                    *control_flow = ControlFlow::Exit;
                } else {
                    eprintln!("FAILURE: One or more security protections failed!");
                    eprintln!("====================================================================\n");
                    process::exit(1);
                }
            }
            Event::UserEvent(IntegrationEvent::Timeout) => {
                eprintln!("\n====================================================================");
                eprintln!("[Integration Test] TIMEOUT: Attack page did not complete in 12s!");
                eprintln!("====================================================================\n");
                process::exit(1);
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
