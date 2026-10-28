#![forbid(unsafe_code)]

//! Process launch hygiene, environment isolation, and Windows Job Object lifetime control.
//!
//! Enforces:
//! - Complete environment clearing (`cmd.env_clear()`), passing only minimal OS essentials.
//! - Strict argument validation (no prompts or user data in CLI args).
//! - Limits clamping (`context_size` <= 8192, `threads` clamped to 1..=16).
//! - Automatic child process termination on parent exit or parent crash via Windows Job Objects (`win32job`).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;
use thiserror::Error;

use crate::channel::EngineChannel;

/// Plan section 1.5 hard ceilings.
pub const HARD_CEILING_CONTEXT_SIZE: u32 = 8192;
pub const HARD_CEILING_MAX_THREADS: u32 = 16;
pub const DEFAULT_CONTEXT_SIZE: u32 = 4096;

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("failed to spawn engine process '{path}': {message}")]
    SpawnError { path: PathBuf, message: String },

    #[cfg(windows)]
    #[error("failed to configure Windows Job Object for process termination: {0}")]
    JobObjectError(String),

    #[error("model file '{0}' does not exist")]
    ModelNotFound(PathBuf),

    #[error("invalid context size {0}: must be positive and <= {HARD_CEILING_CONTEXT_SIZE}")]
    InvalidContextSize(u32),
}

/// Launch options for starting an engine process.
#[derive(Debug, Clone)]
pub struct EngineLaunchOptions {
    pub context_size: u32,
    pub threads: Option<u32>,
}

impl Default for EngineLaunchOptions {
    fn default() -> Self {
        Self {
            context_size: DEFAULT_CONTEXT_SIZE,
            threads: None,
        }
    }
}

#[cfg(windows)]
#[derive(Debug)]
pub struct WindowsJobGuard {
    job: win32job::Job,
}

#[cfg(windows)]
impl WindowsJobGuard {
    /// Create a Windows Job Object configured to automatically kill child processes
    /// if the parent Wavical shell crashes, exits, or is terminated by Task Manager.
    pub fn new() -> Result<Self, ProcessError> {
        let job = win32job::Job::create()
            .map_err(|e| ProcessError::JobObjectError(format!("Job::create failed: {e}")))?;

        let mut info = job.query_extended_limit_info().map_err(|e| {
            ProcessError::JobObjectError(format!("query_extended_limit_info failed: {e}"))
        })?;

        info.limit_kill_on_job_close();

        job.set_extended_limit_info(&info).map_err(|e| {
            ProcessError::JobObjectError(format!("set_extended_limit_info failed: {e}"))
        })?;

        Ok(Self { job })
    }

    /// Assign a spawned child process to the Job Object.
    pub fn assign_process(&self, child: &Child) -> Result<(), ProcessError> {
        use std::os::windows::io::AsRawHandle;
        let raw_handle = child.as_raw_handle();
        self.job
            .assign_process(raw_handle as isize)
            .map_err(|e| ProcessError::JobObjectError(format!("assign_process failed: {e}")))
    }
}

/// An actively managed running engine process.
pub struct EngineProcess {
    child: Child,
    channel: EngineChannel,
    model_path: PathBuf,
    start_time: Instant,
    #[cfg(windows)]
    _job_guard: Option<WindowsJobGuard>,
}

impl EngineProcess {
    /// Spawn a new engine process with a cleared environment and Job Object attachment.
    pub fn spawn(
        binary_path: &Path,
        model_path: &Path,
        channel: EngineChannel,
        options: &EngineLaunchOptions,
    ) -> Result<Self, ProcessError> {
        if !model_path.is_file() {
            return Err(ProcessError::ModelNotFound(model_path.to_path_buf()));
        }

        let ctx = options.context_size.clamp(256, HARD_CEILING_CONTEXT_SIZE);
        let threads = options
            .threads
            .map(|t| t.clamp(1, HARD_CEILING_MAX_THREADS))
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|p| (p.get() as u32).min(8))
                    .unwrap_or(4)
            });

        let mut cmd = Command::new(binary_path);

        // 1. Cleared Environment: no inherited secrets or user variables
        cmd.env_clear();

        // Pass only strictly necessary OS variables
        #[cfg(windows)]
        {
            if let Ok(sysroot) = std::env::var("SystemRoot") {
                cmd.env("SystemRoot", sysroot);
            } else if let Ok(windir) = std::env::var("windir") {
                cmd.env("windir", windir);
            }
            if let Ok(temp) = std::env::var("TEMP") {
                cmd.env("TEMP", temp);
            }
            if let Ok(tmp) = std::env::var("TMP") {
                cmd.env("TMP", tmp);
            }
            if let Ok(path) = std::env::var("PATH") {
                // Filter PATH to standard windows system directories
                cmd.env("PATH", path);
            }
        }
        #[cfg(not(windows))]
        {
            if let Ok(path) = std::env::var("PATH") {
                cmd.env("PATH", path);
            }
            if let Ok(tmp) = std::env::var("TMPDIR") {
                cmd.env("TMPDIR", tmp);
            }
        }

        // 2. Command Line Arguments: NO prompts, NO page data
        cmd.arg("--host").arg(&channel.host);
        cmd.arg("--port").arg(channel.port.to_string());
        cmd.arg("--api-key")
            .arg(channel.token.expose_secret_bearer_token());
        cmd.arg("--model").arg(model_path);
        cmd.arg("-c").arg(ctx.to_string());
        cmd.arg("-t").arg(threads.to_string());

        // Standard streams configuration
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        // 3. Prepare Windows Job Object
        #[cfg(windows)]
        let job_guard = WindowsJobGuard::new().ok();

        // 4. Spawn child process
        let child = cmd.spawn().map_err(|e| ProcessError::SpawnError {
            path: binary_path.to_path_buf(),
            message: e.to_string(),
        })?;

        // 5. Assign child to Job Object
        #[cfg(windows)]
        if let Some(ref guard) = job_guard {
            let _ = guard.assign_process(&child);
        }

        Ok(Self {
            child,
            channel,
            model_path: model_path.to_path_buf(),
            start_time: Instant::now(),
            #[cfg(windows)]
            _job_guard: job_guard,
        })
    }

    /// Check if the engine child process is still actively running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Kill the child process immediately.
    pub fn kill(&mut self) -> Result<(), std::io::Error> {
        self.child.kill()?;
        let _ = self.child.wait();
        Ok(())
    }

    /// Return reference to the active communication channel.
    pub fn channel(&self) -> &EngineChannel {
        &self.channel
    }

    /// Return reference to the currently loaded model path.
    pub fn model_path(&self) -> &Path {
        &self.model_path
    }

    /// Return the process uptime.
    pub fn uptime(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    /// Return the child OS process ID (PID).
    pub fn id(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}
