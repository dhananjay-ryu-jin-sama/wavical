#![no_main]

use libfuzzer_sys::fuzz_target;
use std::sync::atomic::{AtomicBool, Ordering};
use wavical_ipc::{dispatch_raw, Capabilities, HandlerRegistry};

static FORBIDDEN_HANDLER_CALLED: AtomicBool = AtomicBool::new(false);

fn forbidden_handler(_params: &serde_json::Value) -> Result<serde_json::Value, String> {
    FORBIDDEN_HANDLER_CALLED.store(true, Ordering::SeqCst);
    Ok(serde_json::json!("forbidden_data_leaked"))
}

fuzz_target!(|data: &[u8]| {
    FORBIDDEN_HANDLER_CALLED.store(false, Ordering::SeqCst);

    let mut registry = HandlerRegistry::with_core_handlers();
    registry.register("core.forbidden", forbidden_handler);

    // Default-deny: empty capabilities
    let caps = Capabilities::default();

    if let Ok(utf8_str) = std::str::from_utf8(data) {
        let _ = dispatch_raw(utf8_str, &caps, &registry);
    }

    // Security invariant: ungranted method must NEVER execute
    assert!(
        !FORBIDDEN_HANDLER_CALLED.load(Ordering::SeqCst),
        "Ungranted handler was executed by unprivileged input!"
    );
});
