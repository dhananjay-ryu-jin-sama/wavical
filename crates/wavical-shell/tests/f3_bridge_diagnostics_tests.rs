#![forbid(unsafe_code)]

use wavical_netpolicy::NetworkMode;
use wavical_shell::net_policy::{
    BridgeReportOutcome, ShellNetworkPolicy, build_init_script, is_bridge_report_candidate,
    process_bridge_report_message,
};
use wavical_shell::{IpcOriginError, WindowConfig, process_ipc_message};
use wry::http::Uri;

fn app_uri() -> Uri {
    "http://wavical.localhost/index.html"
        .parse::<Uri>()
        .unwrap()
}

#[test]
fn bridge_report_validated_like_any_ipc_message() {
    let mut cfg = WindowConfig::default();
    cfg.network_policy = ShellNetworkPolicy {
        mode: NetworkMode::UltraSafe,
        is_dev_command: true,
        ..Default::default()
    };
    let uri = app_uri();

    // 1. Oversize (> 1 MB) dropped
    let huge = format!(
        r#"{{"id":0,"method":"telemetry.cspBlock","params":{{"directive":"script-src","blockedUri":"{}","topFrame":true}}}}"#,
        "a".repeat(wavical_ipc::MAX_MESSAGE_SIZE_BYTES + 64)
    );
    assert_eq!(
        process_bridge_report_message(&uri, true, &huge, &cfg).unwrap(),
        BridgeReportOutcome::Dropped
    );

    // 2. Null byte dropped
    let null_byte = "{\"id\":0,\"method\":\"telemetry.cspBlock\",\"params\":{\"directive\":\"script-src\",\"blockedUri\":\"https://a.com\0\",\"topFrame\":true}}";
    assert_eq!(
        process_bridge_report_message(&uri, true, null_byte, &cfg).unwrap(),
        BridgeReportOutcome::Dropped
    );

    // 3. Excessive nesting depth (> 32) dropped
    let mut nested = String::from(r#"{"id":0,"method":"telemetry.cspBlock","params":{"extra":"#);
    for _ in 0..40 {
        nested.push('[');
    }
    nested.push('1');
    for _ in 0..40 {
        nested.push(']');
    }
    nested.push_str(r#"}}"#);
    assert_eq!(
        process_bridge_report_message(&uri, true, &nested, &cfg).unwrap(),
        BridgeReportOutcome::Dropped
    );

    // 4. Negative ID / unknown top-level field / unknown params field dropped
    let neg_id = r#"{"id":-1,"method":"telemetry.cspBlock","params":{"directive":"script-src","blockedUri":"https://a.com","topFrame":true}}"#;
    assert_eq!(
        process_bridge_report_message(&uri, true, neg_id, &cfg).unwrap(),
        BridgeReportOutcome::Dropped
    );

    let extra_top = r#"{"id":0,"method":"telemetry.cspBlock","extra":1,"params":{"directive":"script-src","blockedUri":"https://a.com","topFrame":true}}"#;
    assert_eq!(
        process_bridge_report_message(&uri, true, extra_top, &cfg).unwrap(),
        BridgeReportOutcome::Dropped
    );

    let extra_params = r#"{"id":0,"method":"telemetry.cspBlock","params":{"directive":"script-src","blockedUri":"https://a.com","topFrame":true,"secret":1}}"#;
    assert_eq!(
        process_bridge_report_message(&uri, true, extra_params, &cfg).unwrap(),
        BridgeReportOutcome::Dropped
    );

    // 5. Candidate helper detects both report methods cleanly
    assert!(is_bridge_report_candidate(extra_params));
    assert!(is_bridge_report_candidate(
        r#"{"id":0,"method":"dev.report","params":{}}"#
    ));
    assert!(!is_bridge_report_candidate(
        r#"{"id":1,"method":"core.ping"}"#
    ));
}

#[test]
fn bridge_reports_from_iframes_or_other_origins_are_dropped() {
    let mut cfg = WindowConfig::default();
    cfg.network_policy = ShellNetworkPolicy {
        mode: NetworkMode::Open,
        is_dev_command: true,
        ..Default::default()
    };

    let valid_telemetry = r#"{"id":0,"method":"telemetry.cspBlock","params":{"directive":"script-src","blockedUri":"https://cdn.example.com/a.js?q=1","topFrame":true}}"#;

    // Cross-origin and opaque origins rejected
    for bad in [
        "https://evil.com/index.html",
        "https://attacker.localhost/frame.html",
        "http://evil.localhost:8080/a",
        "http://wavical.localhost.evil.com/",
        "http://user:pass@wavical.localhost/",
    ] {
        let bad_uri = bad.parse::<Uri>().unwrap();
        assert!(
            matches!(
                process_bridge_report_message(&bad_uri, true, valid_telemetry, &cfg),
                Err(IpcOriginError::DisallowedOrigin(_))
            ),
            "origin '{bad}' must be rejected"
        );
    }
    for opaque in [
        "about:blank",
        "data:text/html,evil",
        "blob:http://wavical.localhost/uuid",
    ] {
        if let Ok(bad_uri) = opaque.parse::<Uri>() {
            assert!(
                matches!(
                    process_bridge_report_message(&bad_uri, true, valid_telemetry, &cfg),
                    Err(IpcOriginError::DisallowedOrigin(_))
                ),
                "opaque origin '{opaque}' must be rejected"
            );
        }
    }

    // Non-top-frame (iframe) rejected even from app origin
    let uri = app_uri();
    assert!(matches!(
        process_bridge_report_message(&uri, false, valid_telemetry, &cfg),
        Err(IpcOriginError::DisallowedOrigin(_))
    ));

    // topFrame: false in payload dropped
    let iframe_payload = r#"{"id":0,"method":"telemetry.cspBlock","params":{"directive":"script-src","blockedUri":"https://cdn.example.com","topFrame":false}}"#;
    assert_eq!(
        process_bridge_report_message(&uri, true, iframe_payload, &cfg).unwrap(),
        BridgeReportOutcome::Dropped
    );
}

#[test]
fn forwarding_is_off_in_production_shell() {
    let mut prod_cfg = WindowConfig::default();
    prod_cfg.network_policy = ShellNetworkPolicy {
        mode: NetworkMode::UltraSafe,
        is_dev_command: false,
        ..Default::default()
    };
    let uri = app_uri();

    // 1. Production init script contains NO console.error or dev.report hooks
    let init_js = build_init_script(&prod_cfg.network_policy);
    assert!(init_js.contains("telemetry.cspBlock"));
    assert!(!init_js.contains("dev.report"));
    assert!(!init_js.contains("console.error"));
    assert!(!init_js.contains("unhandledrejection"));
    assert!(!init_js.contains("resourceError"));

    // 2. dev.report is NOT forwarded in built apps / production shell and is rejected by IPC
    let dev_report = r#"{"id":1,"method":"dev.report","params":{"kind":"consoleError","message":"leak","location":"app.js:1","topFrame":true}}"#;
    assert_eq!(
        process_bridge_report_message(&uri, true, dev_report, &prod_cfg).unwrap(),
        BridgeReportOutcome::ForwardToIpc
    );
    let registry = wavical_ipc::HandlerRegistry::with_core_handlers();
    let ipc_resp = process_ipc_message(&uri, dev_report, &prod_cfg, &registry)
        .unwrap()
        .unwrap();
    assert!(!ipc_resp.ok);
    assert_eq!(
        ipc_resp.error.as_deref(),
        Some(wavical_ipc::GENERIC_METHOD_ERROR)
    );

    // 3. Built-app telemetry.cspBlock logs via tracing (origin + type) and returns Handled(None) (no forwarding)
    let csp_telemetry = r#"{"id":0,"method":"telemetry.cspBlock","params":{"directive":"script-src-elem","blockedUri":"https://cdn.tailwindcss.com/3.4.0?secret=XYZ","topFrame":true}}"#;
    assert_eq!(
        process_bridge_report_message(&uri, true, csp_telemetry, &prod_cfg).unwrap(),
        BridgeReportOutcome::Handled(None)
    );
}

#[cfg(feature = "dev-shell")]
#[test]
fn dev_shell_validates_and_sanitizes_all_dev_report_kinds() {
    use wavical_netpolicy::diagnostics::{DevDiagnosticEvent, MAX_JS_ERROR_CHARS};

    let mut dev_cfg = WindowConfig::default();
    dev_cfg.network_policy = ShellNetworkPolicy {
        mode: NetworkMode::UltraSafe,
        is_dev_command: true,
        ..Default::default()
    };
    let uri = app_uri();

    let init_js = build_init_script(&dev_cfg.network_policy);
    assert!(init_js.contains("dev.report"));
    assert!(init_js.contains("console.error"));
    assert!(init_js.contains("securitypolicyviolation"));
    assert!(init_js.contains("unhandledrejection"));

    // CSP violation strips query string and maps resource type
    let csp_msg = r#"{"id":0,"method":"dev.report","params":{"kind":"csp","directive":"script-src-elem","blockedUri":"https://cdn.tailwindcss.com/play.js?token=SECRET","resourceType":"script","topFrame":true}}"#;
    assert_eq!(
        process_bridge_report_message(&uri, true, csp_msg, &dev_cfg).unwrap(),
        BridgeReportOutcome::Handled(Some(DevDiagnosticEvent::CspViolation {
            directive: "script-src".to_string(),
            origin: "https://cdn.tailwindcss.com".to_string(),
            resource_type: "script".to_string(),
        }))
    );

    // Uncaught JS error truncates message to 500 chars and strips query from location
    let long_msg = "X".repeat(800);
    let js_msg = format!(
        r#"{{"id":0,"method":"dev.report","params":{{"kind":"jsError","message":"{long_msg}","location":"http://wavical.localhost/app.js?token=SECRET:12","topFrame":true}}}}"#
    );
    match process_bridge_report_message(&uri, true, &js_msg, &dev_cfg).unwrap() {
        BridgeReportOutcome::Handled(Some(DevDiagnosticEvent::JsError { message, location })) => {
            assert!(message.chars().count() <= MAX_JS_ERROR_CHARS);
            assert!(!location.contains("SECRET"));
            assert_eq!(location, "http://wavical.localhost/app.js:12");
        }
        other => panic!("expected JsError, got {other:?}"),
    }

    // console.error and resourceError
    let console_msg = r#"{"id":0,"method":"dev.report","params":{"kind":"consoleError","message":"boom","location":"http://wavical.localhost/main.js?x=1:99","topFrame":true}}"#;
    assert_eq!(
        process_bridge_report_message(&uri, true, console_msg, &dev_cfg).unwrap(),
        BridgeReportOutcome::Handled(Some(DevDiagnosticEvent::ConsoleError {
            message: "boom".to_string(),
            location: "http://wavical.localhost/main.js:99".to_string(),
        }))
    );

    let res_msg = r#"{"id":0,"method":"dev.report","params":{"kind":"resourceError","resourceType":"image","blockedUri":"https://img.example.com/logo.png?sig=SECRET","topFrame":true}}"#;
    assert_eq!(
        process_bridge_report_message(&uri, true, res_msg, &dev_cfg).unwrap(),
        BridgeReportOutcome::Handled(Some(DevDiagnosticEvent::ResourceLoadFailed {
            resource_type: "image".to_string(),
            origin: "https://img.example.com".to_string(),
        }))
    );
}
