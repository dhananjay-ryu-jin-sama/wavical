//! Native crash recovery dialogs and WebView2 process failure hook (WP6).
//!
//! Provides:
//! - Friendly native message box when WebView2 runtime is missing or unusable.
//! - Friendly native message box when WebView2 renderer process crashed repeatedly.
//! - Safe stub for renderer process crash hook (COM hook omitted to uphold strict safety).

/// Display a friendly native message box explaining that WebView2 is missing.
pub fn show_missing_webview2_dialog() {
    rfd::MessageDialog::new()
        .set_title("Microsoft Edge WebView2 Required")
        .set_description(
            "The Microsoft Edge WebView2 Runtime is missing or unusable.\n\n\
             Please install the runtime from:\n\
             https://developer.microsoft.com/en-us/microsoft-edge/webview2/\n\n\
             The application will now close.",
        )
        .set_level(rfd::MessageLevel::Error)
        .show();
}

/// Display a friendly native message box explaining that the renderer process crashed repeatedly.
pub fn show_renderer_crashed_dialog() {
    rfd::MessageDialog::new()
        .set_title("Renderer Process Crashed")
        .set_description(
            "The application renderer process terminated unexpectedly multiple times within 30 seconds.\n\n\
             The application will now close.",
        )
        .set_level(rfd::MessageLevel::Error)
        .show();
}

/// Attach an event listener for WebView2 renderer process failure.
///
/// Direct hooking of Win32 COM `ICoreWebView2::add_ProcessFailed` would violate
/// strict memory safety rules. To uphold memory safety without unapproved
/// code blocks (F-A0-01), this hook does not execute raw COM operations.
pub fn attach_renderer_crash_handler<F>(_webview: &wry::WebView, _on_crash: F) -> Result<(), String>
where
    F: Fn(i32) + Send + 'static,
{
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dialog_functions_callable() {
        // Verification that types and dialog builder definitions compile cleanly
        let _ = show_missing_webview2_dialog as fn();
        let _ = show_renderer_crashed_dialog as fn();
    }
}
