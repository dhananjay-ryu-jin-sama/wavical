//! Single-instance application lock and command-line argument handoff.
//!
//! Uses cross-platform local IPC (`interprocess::local_socket`) backed by Windows Named Pipes
//! or Unix domain sockets.
//!
//! Rules:
//! - Primary instance binds the socket and listens for second launches.
//! - Secondary instance connects, writes its arguments (JSON), flushes, and signals completion.
//! - If the lock check fails (e.g. socket creation error), the app starts normally and logs a warning.
//!   It never crashes.

use interprocess::local_socket::traits::{ListenerExt, Stream as StreamTrait};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, ToNsName};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Role determined for the current process upon single-instance evaluation.
pub enum SingleInstance {
    /// This process is the primary instance and owns the listening socket.
    Primary(SingleInstanceServer),
    /// Another instance is already running; command-line arguments have been forwarded.
    Secondary,
    /// Single-instance check failed or was disabled; proceed as a normal standalone process.
    Disabled,
}

/// Listening server for the primary instance.
pub struct SingleInstanceServer {
    listener: Option<interprocess::local_socket::Listener>,
    stopped: Arc<AtomicBool>,
}

impl SingleInstanceServer {
    /// Spawn a background thread to receive arguments forwarded from second launches.
    pub fn spawn_listener<F>(&mut self, on_new_instance: F)
    where
        F: Fn(Vec<String>) + Send + 'static,
    {
        let Some(listener) = self.listener.take() else {
            return;
        };
        let stopped = Arc::clone(&self.stopped);

        std::thread::Builder::new()
            .name("wavical-single-instance".to_string())
            .spawn(move || {
                for stream_res in listener.incoming() {
                    if stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    match stream_res {
                        Ok(mut stream) => {
                            let mut len_buf = [0u8; 4];
                            if stream.read_exact(&mut len_buf).is_ok() {
                                let len = u32::from_le_bytes(len_buf) as usize;
                                // Max 1 MB for command line arguments payload
                                if len <= 1024 * 1024 {
                                    let mut buf = vec![0u8; len];
                                    if stream.read_exact(&mut buf).is_ok() {
                                        if let Ok(args) = serde_json::from_slice::<Vec<String>>(&buf) {
                                            on_new_instance(args);
                                        }
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            if !stopped.load(Ordering::Relaxed) {
                                tracing::warn!("Single-instance connection error: {err}");
                            }
                            break;
                        }
                    }
                }
            })
            .expect("Failed to spawn single-instance listener thread");
    }

    /// Stop the listener.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}

impl Drop for SingleInstanceServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Compute a secure socket name for the application ID.
pub fn get_socket_name(app_id: &str) -> Result<interprocess::local_socket::Name<'_>, String> {
    let name_str = format!("wavical-{}.sock", app_id);
    name_str
        .to_ns_name::<GenericNamespaced>()
        .map_err(|e| format!("Invalid socket name for appId '{app_id}': {e}"))
}

/// Check single-instance lock for the given `app_id` and forwarded CLI `args`.
///
/// If binding the local socket succeeds: returns `SingleInstance::Primary`.
/// If the socket is already in use: connects, sends `args`, and returns `SingleInstance::Secondary`.
/// If the check encounters an unexpected OS error: logs a warning and returns `SingleInstance::Disabled`.
pub fn check_single_instance(app_id: &str, args: &[String]) -> Result<SingleInstance, String> {
    let name = match get_socket_name(app_id) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("Failed to generate socket name: {e}; starting normally");
            return Ok(SingleInstance::Disabled);
        }
    };

    let opts = ListenerOptions::new().name(name.clone());
    match opts.create_sync() {
        Ok(listener) => Ok(SingleInstance::Primary(SingleInstanceServer {
            listener: Some(listener),
            stopped: Arc::new(AtomicBool::new(false)),
        })),
        Err(err) => {
            if err.kind() == std::io::ErrorKind::AddrInUse
                || err.kind() == std::io::ErrorKind::AlreadyExists
                || err.kind() == std::io::ErrorKind::PermissionDenied
            {
                // Attempt to connect to the primary instance
                match Stream::connect(name) {
                    Ok(mut stream) => {
                        let payload = serde_json::to_vec(args).unwrap_or_default();
                        let len_bytes = (payload.len() as u32).to_le_bytes();
                        if stream.write_all(&len_bytes).is_ok()
                            && stream.write_all(&payload).is_ok()
                            && stream.flush().is_ok()
                        {
                            return Ok(SingleInstance::Secondary);
                        } else {
                            tracing::warn!("Failed to write args to existing instance; starting normally");
                            Ok(SingleInstance::Disabled)
                        }
                    }
                    Err(connect_err) => {
                        tracing::warn!(
                            "Existing socket found but failed to connect ({connect_err}); starting normally"
                        );
                        Ok(SingleInstance::Disabled)
                    }
                }
            } else {
                tracing::warn!(
                    "Single-instance lock check failed ({err}); starting normally"
                );
                Ok(SingleInstance::Disabled)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn test_single_instance_handoff() {
        let unique_app_id = format!("test-si-{}", std::process::id());
        let initial_args = vec!["--first".to_string()];

        // 1. First launch -> Primary
        let primary = check_single_instance(&unique_app_id, &initial_args)
            .expect("first instance check should succeed");

        let SingleInstance::Primary(mut server) = primary else {
            panic!("First instance must be Primary");
        };

        let (tx, rx) = mpsc::channel();
        server.spawn_listener(move |args| {
            let _ = tx.send(args);
        });

        // 2. Second launch -> Secondary with forwarded arguments
        let secondary_args = vec!["--open".to_string(), "file.txt".to_string()];
        let secondary = check_single_instance(&unique_app_id, &secondary_args)
            .expect("second instance check should succeed");

        assert!(
            matches!(secondary, SingleInstance::Secondary),
            "Second instance must be Secondary"
        );

        // 3. Primary receives the forwarded arguments from secondary
        let received = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("primary should receive arguments within timeout");
        assert_eq!(received, secondary_args);
    }
}
