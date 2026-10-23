//! Native crash recovery dialogs and WebView2 process failure hook (WP6).
//!
//! Provides:
//! - Friendly native message box when WebView2 runtime is missing or unusable.
//! - Friendly native message box when WebView2 renderer process crashed repeatedly.
//! - Native hook on Windows hooking `ICoreWebView2::add_ProcessFailed`.

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

/// Attach an event listener for WebView2 renderer process failure on Windows.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
pub fn attach_renderer_crash_handler<F>(webview: &wry::WebView, on_crash: F) -> Result<(), String>
where
    F: Fn(i32) + Send + 'static,
{
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use webview2_com::ProcessFailedEventHandler;
    use wry::WebViewExtWindows;

    let core_wv = webview.webview();
    let mut token = 0i64;
    let handler = ProcessFailedEventHandler::create(Box::new(move |_sender, args| {
        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND(0);
        if let Some(ref args) = args {
            // SAFETY: args is a valid ICoreWebView2ProcessFailedEventArgs and kind is a valid pointer.
            unsafe {
                let _ = args.ProcessFailedKind(&mut kind);
            }
        }
        on_crash(kind.0);
        Ok(())
    }));

    // SAFETY: core_wv is a valid ICoreWebView2 instance provided by wry,
    // handler implements ICoreWebView2ProcessFailedEventHandler via webview2-com,
    // and token points to a valid local EventRegistrationToken on the stack.
    let res = unsafe { core_wv.add_ProcessFailed(&handler, &mut token) };
    res.map_err(|e| format!("Failed to register ProcessFailed event listener: {e}"))?;
    Ok(())
}

/// Fallback for non-Windows targets.
#[cfg(not(target_os = "windows"))]
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
