#![forbid(unsafe_code)]

//! Lifecycle manager orchestrating engine startup, idle shutdown, crash recovery, and health verification.

use reqwest::blocking::Client;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use thiserror::Error;

use crate::channel::{ChannelError, EngineChannel};
use crate::client::{
    ClientError, DEFAULT_HEALTH_CHECK_TIMEOUT, create_engine_http_client, wait_for_engine_ready,
};
use crate::engine::{EngineError, expected_engine_hash, resolve_engine_path, verify_engine_binary};
use crate::lifecycle_policy::{
    CrashLimitExceeded, CrashRecoveryPolicy, DEFAULT_IDLE_MINUTES, IdleShutdownPolicy,
};
use crate::process::{EngineLaunchOptions, EngineProcess, ProcessError};

#[derive(Debug, Error)]
pub enum ManagerError {
    #[error("engine verification failed: {0}")]
    EngineVerification(#[from] EngineError),

    #[error("failed to create communication channel: {0}")]
    Channel(#[from] ChannelError),

    #[error("failed to spawn engine process: {0}")]
    Process(#[from] ProcessError),

    #[error("engine client error: {0}")]
    Client(#[from] ClientError),

    #[error("engine crash limit exceeded: {0}")]
    CrashLimit(#[from] CrashLimitExceeded),
}

/// The local AI engine manager.
pub struct LocalEngineManager {
    binary_path: PathBuf,
    expected_hash: String,
    active_process: Option<EngineProcess>,
    idle_policy: IdleShutdownPolicy,
    crash_policy: CrashRecoveryPolicy,
    launch_options: EngineLaunchOptions,
    http_client: Client,
    health_timeout: Duration,
}

impl LocalEngineManager {
    /// Create a new engine manager with default settings.
    pub fn new() -> Result<Self, ManagerError> {
        let binary_path = resolve_engine_path();
        let expected_hash = expected_engine_hash();
        let idle_policy = IdleShutdownPolicy::new(DEFAULT_IDLE_MINUTES, Instant::now());
        let crash_policy = CrashRecoveryPolicy::new();
        let launch_options = EngineLaunchOptions::default();
        let http_client = create_engine_http_client()?;

        Ok(Self {
            binary_path,
            expected_hash,
            active_process: None,
            idle_policy,
            crash_policy,
            launch_options,
            http_client,
            health_timeout: DEFAULT_HEALTH_CHECK_TIMEOUT,
        })
    }

    /// Create an engine manager with explicit configuration for testing.
    pub fn with_config(
        binary_path: PathBuf,
        expected_hash: String,
        idle_minutes: u32,
        launch_options: EngineLaunchOptions,
        health_timeout: Duration,
    ) -> Result<Self, ManagerError> {
        let idle_policy = IdleShutdownPolicy::new(idle_minutes, Instant::now());
        let crash_policy = CrashRecoveryPolicy::new();
        let http_client = create_engine_http_client()?;

        Ok(Self {
            binary_path,
            expected_hash,
            active_process: None,
            idle_policy,
            crash_policy,
            launch_options,
            http_client,
            health_timeout,
        })
    }

    /// Ensure that an engine instance is actively running and ready to serve requests for `model_path`.
    ///
    /// - Starts lazily if not currently running.
    /// - Verifies engine binary hash before every spawn.
    /// - Restarts on crash subject to the crash retry limiter.
    /// - Replaces engine instance if requested model differs from currently loaded model.
    /// - Refreshes the idle countdown.
    pub fn ensure_running(&mut self, model_path: &Path) -> Result<EngineChannel, ManagerError> {
        let now = Instant::now();

        // 1. Check existing process if present
        let needs_start = match self.active_process {
            Some(ref mut proc) => {
                if proc.is_alive() {
                    if proc.model_path() == model_path {
                        self.idle_policy.record_activity(now);
                        let uptime = proc.uptime();
                        self.crash_policy.reset_if_stable(uptime);
                        false
                    } else {
                        let _ = proc.kill();
                        true
                    }
                } else {
                    let cooldown = self.crash_policy.record_crash_and_check_restart(now)?;
                    if !cooldown.is_zero() {
                        std::thread::sleep(cooldown);
                    }
                    true
                }
            }
            None => true,
        };

        if needs_start {
            self.active_process = None;

            // 2. Hash check BEFORE every launch
            verify_engine_binary(&self.binary_path, &self.expected_hash)?;

            // 3. Setup fresh ephemeral channel
            let channel = EngineChannel::new_ephemeral()?;

            // 4. Launch engine process with sanitized environment
            let proc =
                EngineProcess::spawn(&self.binary_path, model_path, channel, &self.launch_options)?;

            // 5. Handshake & health check
            wait_for_engine_ready(&self.http_client, proc.channel(), self.health_timeout)?;

            // 6. Record activity
            self.idle_policy.record_activity(now);
            self.active_process = Some(proc);
        }

        Ok(self.active_process.as_ref().unwrap().channel().clone())
    }

    /// Check if the engine should be shut down due to inactivity.
    ///
    /// If idle, kills the process and frees all memory. Returns true if shutdown occurred.
    pub fn check_idle_shutdown(&mut self, now: Instant) -> bool {
        if let Some(ref mut proc) = self.active_process
            && proc.is_alive()
            && self.idle_policy.is_idle(now)
        {
            let _ = proc.kill();
            self.active_process = None;
            return true;
        }
        false
    }

    /// Gracefully terminate any running engine process.
    pub fn shutdown(&mut self) {
        if let Some(mut proc) = self.active_process.take() {
            let _ = proc.kill();
        }
    }

    /// Check if an engine process is currently active and alive.
    pub fn is_running(&mut self) -> bool {
        match self.active_process {
            Some(ref mut p) => p.is_alive(),
            None => false,
        }
    }

    /// Return reference to the active channel if currently running.
    pub fn active_channel(&self) -> Option<&EngineChannel> {
        self.active_process.as_ref().map(|p| p.channel())
    }

    /// Return the remaining time before idle shutdown triggers.
    pub fn idle_remaining(&self, now: Instant) -> Duration {
        self.idle_policy.remaining_time(now)
    }
}

impl Drop for LocalEngineManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}
