#![forbid(unsafe_code)]

//! Native first-use cloud consent dialog and persistence (Phase 3 WP7).
//!
//! Enforces:
//! - First cloud use shows a native dialog naming the provider host and stating
//!   that the user's text will be sent to that service.
//! - The decision is stored per app and host under `<app_data>/apps/<app_id>/ai/cloud_consent.json`.
//! - Atomic write via temporary file replacement.
//! - The page cannot trigger, skip, or read consent.
//! - Declining stores negative consent and returns generic error.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Error type for consent operations.
#[derive(Debug, Error)]
pub enum ConsentError {
    #[error("cloud consent was declined by user")]
    Denied,

    #[error("failed to read consent storage: {0}")]
    IoReadError(String),

    #[error("failed to write consent storage: {0}")]
    IoWriteError(String),

    #[error("invalid appId or provider host")]
    InvalidParameters,
}

/// Persistent consent storage schema.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CloudConsentData {
    /// Mapping from provider host (e.g. "api.openai.com") to granted boolean.
    #[serde(default)]
    pub consents: HashMap<String, bool>,
}

/// Abstract provider for showing the native consent dialog.
///
/// Allows injecting a mock provider during automated testing to avoid blocking on GUI.
pub trait ConsentDialogProvider: Send + Sync {
    /// Present confirmation dialog naming the app and provider host.
    /// Returns `true` if user accepted, `false` if user declined.
    fn prompt_consent(&self, app_id: &str, provider_host: &str) -> bool;
}

/// Default native implementation using RFD message dialog.
pub struct NativeConsentDialogProvider;

impl ConsentDialogProvider for NativeConsentDialogProvider {
    fn prompt_consent(&self, app_id: &str, provider_host: &str) -> bool {
        let title = "Wavical — Cloud AI Confirmation";
        let description = format!(
            "App \"{app_id}\" wants to use cloud AI provided by \"{provider_host}\".\n\n\
             Your input text and conversation history will be sent to that service.\n\n\
             Do you consent to sending your data to {provider_host}?"
        );

        let result = rfd::MessageDialog::new()
            .set_title(title)
            .set_description(&description)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();

        matches!(result, rfd::MessageDialogResult::Yes)
    }
}

/// Mock dialog provider for unit/integration tests.
pub struct MockConsentDialogProvider {
    pub allow: bool,
}

impl ConsentDialogProvider for MockConsentDialogProvider {
    fn prompt_consent(&self, _app_id: &str, _provider_host: &str) -> bool {
        self.allow
    }
}

/// Resolve consent file path for an application.
pub fn get_consent_file_path(app_data_path: &Path, app_id: &str) -> PathBuf {
    app_data_path
        .join("apps")
        .join(app_id)
        .join("ai")
        .join("cloud_consent.json")
}

/// Check existing recorded consent for an app and provider host.
///
/// Returns:
/// - `Some(true)` if previously accepted.
/// - `Some(false)` if previously declined.
/// - `None` if no consent decision has been recorded yet.
pub fn check_cloud_consent(
    app_data_path: &Path,
    app_id: &str,
    provider_host: &str,
) -> Option<bool> {
    let path = get_consent_file_path(app_data_path, app_id);
    if !path.exists() {
        return None;
    }

    let bytes = std::fs::read(&path).ok()?;
    let data: CloudConsentData = serde_json::from_slice(&bytes).ok()?;
    data.consents.get(provider_host).copied()
}

/// Record consent decision for an app and provider host atomically.
pub fn record_cloud_consent(
    app_data_path: &Path,
    app_id: &str,
    provider_host: &str,
    granted: bool,
) -> Result<(), ConsentError> {
    let path = get_consent_file_path(app_data_path, app_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ConsentError::IoWriteError(format!("create dir failed: {e}")))?;
    }

    let mut data = if path.exists() {
        let bytes = std::fs::read(&path)
            .map_err(|e| ConsentError::IoReadError(format!("read failed: {e}")))?;
        serde_json::from_slice(&bytes).unwrap_or_default()
    } else {
        CloudConsentData::default()
    };

    data.consents.insert(provider_host.to_string(), granted);

    let serialized = serde_json::to_vec_pretty(&data)
        .map_err(|e| ConsentError::IoWriteError(format!("serialize failed: {e}")))?;

    // Atomic write via tempfile in the same directory
    let temp_path = path.with_extension("tmp");
    std::fs::write(&temp_path, serialized)
        .map_err(|e| ConsentError::IoWriteError(format!("write temp failed: {e}")))?;
    std::fs::rename(&temp_path, &path)
        .map_err(|e| ConsentError::IoWriteError(format!("rename failed: {e}")))?;

    Ok(())
}

/// Ensure cloud consent is satisfied for a request.
///
/// If consent has not been given yet, presents the native dialog via `dialog_provider`.
/// If user declines, records `false` and returns `Err(ConsentError::Denied)`.
/// If user accepts, records `true` and returns `Ok(())`.
pub fn ensure_cloud_consent(
    app_data_path: &Path,
    app_id: &str,
    provider_host: &str,
    require_consent_config: bool,
    dialog_provider: &dyn ConsentDialogProvider,
) -> Result<(), ConsentError> {
    if !require_consent_config {
        return Ok(());
    }

    match check_cloud_consent(app_data_path, app_id, provider_host) {
        Some(true) => Ok(()),
        Some(false) => Err(ConsentError::Denied),
        None => {
            let accepted = dialog_provider.prompt_consent(app_id, provider_host);
            record_cloud_consent(app_data_path, app_id, provider_host, accepted)?;
            if accepted {
                Ok(())
            } else {
                Err(ConsentError::Denied)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_cloud_consent_lifecycle() {
        let temp = tempdir().expect("tempdir");
        let app_data = temp.path();
        let app_id = "test-app";
        let host = "api.openai.com";

        // Initially no consent
        assert_eq!(check_cloud_consent(app_data, app_id, host), None);

        // Deny prompt
        let mock_deny = MockConsentDialogProvider { allow: false };
        let res_deny = ensure_cloud_consent(app_data, app_id, host, true, &mock_deny);
        assert!(matches!(res_deny, Err(ConsentError::Denied)));
        assert_eq!(check_cloud_consent(app_data, app_id, host), Some(false));

        // Subsequent check fails without prompting
        let res_subsequent = ensure_cloud_consent(app_data, app_id, host, true, &mock_deny);
        assert!(matches!(res_subsequent, Err(ConsentError::Denied)));

        // Overwrite with grant
        record_cloud_consent(app_data, app_id, host, true).expect("record grant");
        assert_eq!(check_cloud_consent(app_data, app_id, host), Some(true));

        let res_granted = ensure_cloud_consent(app_data, app_id, host, true, &mock_deny);
        assert!(res_granted.is_ok());

        // Other host still has no consent
        assert_eq!(
            check_cloud_consent(app_data, app_id, "api.anthropic.com"),
            None
        );
    }
}
