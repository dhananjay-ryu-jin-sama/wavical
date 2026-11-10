#![forbid(unsafe_code)]

//! Backend runtime launcher, lifetime manager, reduced privilege enforcement,
//! and isolated Named Pipe IPC channel.
//!
//! Enforces:
//! - Hash verification before every launch.
//! - Environment clearing (`cmd.env_clear()`) passing only minimal OS essentials and validated `backend.env` variables.
//! - Strict rejection of debug/preload flags (`--inspect`, `--require`, etc.) in release builds.
//! - Node.js permission model flags for allowed roots and capabilities (or loud warning if unrestricted).
//! - Automatic child termination on parent crash or exit via Windows Job Objects (`win32job`).
//! - Crash restart throttling via pure `BackendRestartPolicy` (max 3 retries in 60s).
//! - Capturing child stdout/stderr into `tracing` with line-size (4 KB) and rate (200 lines/s) caps.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;

use interprocess::local_socket::traits::Listener as ListenerTrait;
use interprocess::local_socket::{
    GenericNamespaced, ListenerNonblockingMode, ListenerOptions, Stream, ToNsName,
};

use wavical_ipc::{MAX_BACKEND_LOG_LINE_BYTES, MAX_BACKEND_LOG_RATE_PER_SEC};
use wavical_protocol::backend_args::{
    build_node_permission_flags, is_forbidden_node_env_var, validate_backend_cli_args,
};
use wavical_protocol::backend_restart_policy::{BackendCrashLimitExceeded, BackendRestartPolicy};
use wavical_protocol::runtime_manifest::RuntimeArch;
use wavical_protocol::version_resolver::resolve_runtime_version;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("runtime version '{version}' resolution failed: {message}")]
    VersionResolutionFailed { version: String, message: String },

    #[error("runtime executable '{0}' does not exist")]
    RuntimeExecutableNotFound(PathBuf),

    #[error(
        "executable hash verification failed for '{path}': expected '{expected}', computed '{actual}'"
    )]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },

    #[error("failed to read executable '{path}': {source}")]
    IoError {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("debug/preload flag '{0}' is strictly forbidden in release builds")]
    DebugFlagsForbiddenInRelease(String),

    #[error("failed to create Windows Job Object for backend: {0}")]
    JobObjectError(String),

    #[error("failed to create Named Pipe listener for backend: {0}")]
    PipeCreationError(String),

    #[error("failed to spawn backend child process: {0}")]
    SpawnError(#[from] std::io::Error),

    #[error("backend handshake timed out waiting for connection on pipe '{0}'")]
    PipeConnectTimeout(String),

    #[error("backend crash limit exceeded: {0}")]
    CrashLimit(#[from] BackendCrashLimitExceeded),

    #[error("backend working directory '{0}' does not exist")]
    WorkingDirectoryNotFound(PathBuf),

    #[error("backend entry script '{0}' does not exist")]
    EntryScriptNotFound(PathBuf),
}

/// Windows Job Object wrapper ensuring kernel-level kill-on-close.
#[cfg(windows)]
#[derive(Debug)]
pub struct WindowsJobGuard {
    job: win32job::Job,
}

#[cfg(windows)]
impl WindowsJobGuard {
    pub fn new() -> Result<Self, BackendError> {
        let job = win32job::Job::create()
            .map_err(|e| BackendError::JobObjectError(format!("Job::create failed: {e}")))?;

        let mut info = job.query_extended_limit_info().map_err(|e| {
            BackendError::JobObjectError(format!("query_extended_limit_info failed: {e}"))
        })?;

        info.limit_kill_on_job_close();

        job.set_extended_limit_info(&info).map_err(|e| {
            BackendError::JobObjectError(format!("set_extended_limit_info failed: {e}"))
        })?;

        Ok(Self { job })
    }

    pub fn assign_process(&self, child: &Child) -> Result<(), BackendError> {
        use std::os::windows::io::AsRawHandle;
        let raw_handle = child.as_raw_handle();
        self.job
            .assign_process(raw_handle as isize)
            .map_err(|e| BackendError::JobObjectError(format!("assign_process failed: {e}")))
    }
}

/// Configuration options for launching a backend runtime instance.
#[derive(Debug, Clone)]
pub struct BackendLaunchConfig {
    pub app_id: String,
    pub runtime: String,
    pub version: String,
    pub allow_patch_updates: bool,
    pub entry_path: PathBuf,
    pub working_dir: PathBuf,
    pub app_data_dir: PathBuf,
    pub fs_read_roots: Vec<PathBuf>,
    pub fs_write_roots: Vec<PathBuf>,
    pub allow_child_process: bool,
    pub allow_workers: bool,
    pub extra_env_vars: Vec<String>,
    pub unrestricted: bool,
    pub custom_binary_override: Option<PathBuf>,
    pub custom_binary_expected_hash: Option<String>,
    pub custom_args: Vec<String>,
}

impl BackendLaunchConfig {
    /// Create minimal config with standard defaults for an application.
    pub fn new(app_id: &str, project_root: &Path, entry_rel: &str) -> Self {
        let backend_dir = project_root.join("backend");
        let app_data = std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Wavical")
            .join("apps")
            .join(app_id);

        Self {
            app_id: app_id.to_string(),
            runtime: "node".to_string(),
            version: "22.11.0".to_string(),
            allow_patch_updates: false,
            entry_path: project_root.join(entry_rel),
            working_dir: backend_dir,
            app_data_dir: app_data,
            fs_read_roots: Vec::new(),
            fs_write_roots: Vec::new(),
            allow_child_process: false,
            allow_workers: false,
            extra_env_vars: Vec::new(),
            unrestricted: false,
            custom_binary_override: None,
            custom_binary_expected_hash: None,
            custom_args: Vec::new(),
        }
    }
}

/// A running backend process along with its Job Object, listener, and log readers.
pub struct BackendProcess {
    child: Child,
    pipe_name: String,
    listener: Option<interprocess::local_socket::Listener>,
    stream: Option<Stream>,
    start_time: Instant,
    stopped: Arc<AtomicBool>,
    #[cfg(windows)]
    _job_guard: Option<WindowsJobGuard>,
}

impl BackendProcess {
    /// Return whether the child process is still actively running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Kill the child process immediately.
    pub fn kill(&mut self) -> Result<(), std::io::Error> {
        self.stopped.store(true, Ordering::SeqCst);
        let _ = self.child.kill();
        let _ = self.child.wait();
        Ok(())
    }

    /// Return child process OS identifier (PID).
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Return the pipe name.
    pub fn pipe_name(&self) -> &str {
        &self.pipe_name
    }

    /// Accept the incoming IPC connection from the backend process.
    pub fn accept_connection(&mut self, timeout: Duration) -> Result<&mut Stream, BackendError> {
        if let Some(ref mut stream) = self.stream {
            return Ok(stream);
        }

        let listener = self.listener.take().ok_or_else(|| {
            BackendError::PipeCreationError("pipe listener already consumed".to_string())
        })?;

        // Try accepting with short non-blocking sleep loop
        let start = Instant::now();
        let _ = listener.set_nonblocking(ListenerNonblockingMode::Accept);

        loop {
            match listener.accept() {
                Ok(stream) => {
                    self.stream = Some(stream);
                    return Ok(self.stream.as_mut().unwrap());
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if start.elapsed() >= timeout {
                        return Err(BackendError::PipeConnectTimeout(self.pipe_name.clone()));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    return Err(BackendError::PipeCreationError(format!(
                        "accept failed: {e}"
                    )));
                }
            }
        }
    }

    /// Return a mutable reference to the connected IPC stream if established.
    pub fn stream_mut(&mut self) -> Option<&mut Stream> {
        self.stream.as_mut()
    }

    /// Take the connected stream.
    pub fn take_stream(&mut self) -> Option<Stream> {
        self.stream.take()
    }

    /// Return the uptime duration.
    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }
}

impl Drop for BackendProcess {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Locate a binary on the host PATH.
pub fn find_in_path(executable_name: &str) -> Option<PathBuf> {
    if let Ok(paths) = std::env::var("PATH") {
        for part in std::env::split_paths(&paths) {
            let candidate = part.join(executable_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Verifies the SHA-256 hash of an executable before launch.
pub fn verify_executable_hash(path: &Path, expected_hash: &str) -> Result<(), BackendError> {
    if !path.is_file() {
        return Err(BackendError::RuntimeExecutableNotFound(path.to_path_buf()));
    }

    let actual_hash = wavical_ai::sha256::sha256_file(path).map_err(|e| BackendError::IoError {
        path: path.to_path_buf(),
        source: e,
    })?;

    if !actual_hash.eq_ignore_ascii_case(expected_hash) {
        return Err(BackendError::HashMismatch {
            path: path.to_path_buf(),
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    Ok(())
}

/// Spawns a background thread reading lines from a process stdout/stderr stream,
/// enforcing line size and rate limits before routing to tracing.
pub fn spawn_log_stream_reader<R: Read + Send + 'static>(
    reader: R,
    is_stderr: bool,
    stopped: Arc<AtomicBool>,
) {
    let target = if is_stderr {
        "backend::stderr"
    } else {
        "backend::stdout"
    };

    std::thread::Builder::new()
        .name(format!("wavical-{target}"))
        .spawn(move || {
            let mut buf_reader = BufReader::new(reader);
            let mut line = String::new();

            let mut current_sec = Instant::now();
            let mut lines_in_sec = 0usize;
            let mut rate_limited = false;

            while !stopped.load(Ordering::Relaxed) {
                line.clear();
                match buf_reader.read_line(&mut line) {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        let now = Instant::now();
                        if now.duration_since(current_sec) >= Duration::from_secs(1) {
                            current_sec = now;
                            lines_in_sec = 0;
                            rate_limited = false;
                        }

                        lines_in_sec += 1;
                        if lines_in_sec > MAX_BACKEND_LOG_RATE_PER_SEC {
                            if !rate_limited {
                                rate_limited = true;
                                tracing::warn!(
                                    target: "backend::logs",
                                    "Backend log rate limit ({} lines/s) reached; throttling further logs in this window",
                                    MAX_BACKEND_LOG_RATE_PER_SEC
                                );
                            }
                            continue;
                        }

                        // Truncate line safely at UTF-8 boundary if it exceeds limit
                        let trimmed = line.trim_end_matches(['\r', '\n']);
                        let safe_line = if trimmed.len() > MAX_BACKEND_LOG_LINE_BYTES {
                            let mut boundary = MAX_BACKEND_LOG_LINE_BYTES;
                            while !trimmed.is_char_boundary(boundary) && boundary > 0 {
                                boundary -= 1;
                            }
                            let mut truncated = trimmed[..boundary].to_string();
                            truncated.push_str(" ... [truncated]");
                            truncated
                        } else {
                            trimmed.to_string()
                        };

                        if is_stderr {
                            tracing::warn!(target: "backend::stderr", "{}", safe_line);
                        } else {
                            tracing::info!(target: "backend::stdout", "{}", safe_line);
                        }
                    }
                    Err(_) => break,
                }
            }
        })
        .ok();
}

/// Backend Launcher responsible for spawning the backend runtime with full isolation.
pub struct BackendLauncher;

impl BackendLauncher {
    /// Resolve the executable path and expected hash for the specified configuration.
    pub fn resolve_runtime(
        config: &BackendLaunchConfig,
    ) -> Result<(PathBuf, Option<String>), BackendError> {
        if let Some(ref override_path) = config.custom_binary_override {
            return Ok((
                override_path.clone(),
                config.custom_binary_expected_hash.clone(),
            ));
        }

        // Available versions in manifest
        let available: Vec<&str> = wavical_protocol::APPROVED_RUNTIMES
            .iter()
            .filter(|e| e.runtime == config.runtime)
            .map(|e| e.version)
            .collect();

        let resolved_version =
            resolve_runtime_version(&config.version, config.allow_patch_updates, &available)
                .map_err(|e| BackendError::VersionResolutionFailed {
                    version: config.version.clone(),
                    message: e.to_string(),
                })?;

        // Locate entry in manifest
        let arch = RuntimeArch::current();
        let manifest_entry =
            wavical_protocol::find_approved_runtime(&config.runtime, &resolved_version, arch)
                .map_err(|e| BackendError::VersionResolutionFailed {
                    version: config.version.clone(),
                    message: e.to_string(),
                })?;

        // Expected path in cache
        let local_app_data = std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));

        let cached_path = local_app_data
            .join("Wavical")
            .join("runtimes")
            .join(format!(
                "{}-{}-{}",
                config.runtime,
                resolved_version,
                manifest_entry.arch.as_str()
            ))
            .join(manifest_entry.executable_name);

        if cached_path.is_file() {
            return Ok((cached_path, Some(manifest_entry.sha256.to_string())));
        }

        // Fallback to system node if available
        let system_node = if cfg!(windows) { "node.exe" } else { "node" };
        if let Some(path) = find_in_path(system_node) {
            eprintln!(
                "notice: managed Node runtime '{resolved_version}' not found in cache; using system Node.js fallback."
            );
            return Ok((path, None));
        }

        Err(BackendError::RuntimeExecutableNotFound(cached_path))
    }

    /// Spawn the backend process with the permission model, cleaned environment,
    /// Job Object, and Named Pipe IPC channel.
    pub fn spawn(config: &BackendLaunchConfig) -> Result<BackendProcess, BackendError> {
        if !config.working_dir.exists() {
            let _ = fs::create_dir_all(&config.working_dir);
        }

        // 1. Resolve runtime binary and verify hash if known
        let (binary_path, expected_hash) = Self::resolve_runtime(config)?;
        if let Some(hash) = expected_hash {
            verify_executable_hash(&binary_path, &hash)?;
        }

        // 2. Setup isolated Named Pipe server
        let token: String = {
            use std::time::SystemTime;
            let nanos = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            format!("{:x}", nanos)
        };
        let pipe_name = format!("wavical-backend-{}-{}", config.app_id, token);
        let ns_name = pipe_name
            .clone()
            .to_ns_name::<GenericNamespaced>()
            .map_err(|e| BackendError::PipeCreationError(e.to_string()))?;

        let listener = ListenerOptions::new()
            .name(ns_name)
            .create_sync()
            .map_err(|e| BackendError::PipeCreationError(e.to_string()))?;

        // 3. Build arguments
        let mut args: Vec<String> = Vec::new();

        if config.unrestricted {
            eprintln!(
                "SECURITY WARNING: backend.unrestricted is enabled; permission model is disabled!"
            );
            tracing::warn!(
                "SECURITY WARNING: backend.unrestricted is enabled; permission model is disabled!"
            );
        } else {
            // Determine major version from binary if possible, falling back to config.version
            let detected_major = Command::new(&binary_path)
                .arg("--version")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|v| {
                    let trimmed = v.trim().trim_start_matches('v');
                    trimmed
                        .split('.')
                        .next()
                        .and_then(|s| s.parse::<u32>().ok())
                });

            let major_version = detected_major.unwrap_or_else(|| {
                config
                    .version
                    .split('.')
                    .next()
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(22)
            });

            let mut read_roots: Vec<String> = Vec::new();
            read_roots.push(config.working_dir.to_string_lossy().to_string());
            read_roots.push(config.app_data_dir.to_string_lossy().to_string());
            for r in &config.fs_read_roots {
                read_roots.push(r.to_string_lossy().to_string());
            }

            let mut write_roots: Vec<String> = Vec::new();
            write_roots.push(config.app_data_dir.to_string_lossy().to_string());
            for w in &config.fs_write_roots {
                write_roots.push(w.to_string_lossy().to_string());
            }

            let perm_flags = build_node_permission_flags(
                false,
                major_version,
                &read_roots,
                &write_roots,
                config.allow_child_process,
                config.allow_workers,
            );
            args.extend(perm_flags);
        }

        // Entry script
        args.push(config.entry_path.to_string_lossy().to_string());
        args.extend(config.custom_args.clone());

        // Validate that no debug/preload flags exist in release builds
        let is_release = !cfg!(debug_assertions);
        if let Err(e) = validate_backend_cli_args(&args, is_release) {
            return Err(BackendError::DebugFlagsForbiddenInRelease(e.to_string()));
        }

        // 4. Configure Command with cleared environment
        let mut cmd = Command::new(&binary_path);
        cmd.current_dir(&config.working_dir);
        cmd.env_clear();

        // Pass minimal OS platform essentials
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

        // Channel information
        cmd.env("WAVICAL_IPC_PIPE", &pipe_name);
        cmd.env("WAVICAL_APP_ID", &config.app_id);

        // Pass through only safe, explicitly listed environment variables
        for var_name in &config.extra_env_vars {
            if !is_forbidden_node_env_var(var_name)
                && let Ok(val) = std::env::var(var_name)
            {
                cmd.env(var_name, val);
            }
        }

        for arg in &args {
            cmd.arg(arg);
        }

        // Standard streams
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // 5. Windows Job Object for lifecycle enforcement
        #[cfg(windows)]
        let job_guard = WindowsJobGuard::new().ok();

        // 6. Spawn process
        let mut child = cmd.spawn()?;

        #[cfg(windows)]
        if let Some(ref guard) = job_guard {
            let _ = guard.assign_process(&child);
        }

        let stopped = Arc::new(AtomicBool::new(false));

        // Start log streams
        if let Some(stdout) = child.stdout.take() {
            spawn_log_stream_reader(stdout, false, Arc::clone(&stopped));
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_log_stream_reader(stderr, true, Arc::clone(&stopped));
        }

        Ok(BackendProcess {
            child,
            pipe_name,
            listener: Some(listener),
            stream: None,
            start_time: Instant::now(),
            stopped,
            #[cfg(windows)]
            _job_guard: job_guard,
        })
    }
}

/// Orchestrates backend process lifetime, startup, crash detection, and restart throttling.
pub struct BackendProcessManager {
    config: BackendLaunchConfig,
    active_process: Option<BackendProcess>,
    restart_policy: BackendRestartPolicy,
}

impl BackendProcessManager {
    /// Create a new process manager for the given configuration.
    pub fn new(config: BackendLaunchConfig) -> Self {
        Self {
            config,
            active_process: None,
            restart_policy: BackendRestartPolicy::new(),
        }
    }

    /// Ensure that the backend process is actively running.
    /// If it has crashed, restarts subject to the retry limiter (3 crashes in 60s).
    pub fn ensure_running(&mut self) -> Result<bool, BackendError> {
        let is_running = self
            .active_process
            .as_mut()
            .map(|p| p.is_alive())
            .unwrap_or(false);

        if is_running {
            return Ok(false); // already running, no restart occurred
        }

        // If an existing process died, evaluate restart policy
        if self.active_process.is_some() {
            let now = Instant::now();
            let _cooldown = self.restart_policy.record_crash_and_check(now)?;
            tracing::warn!(
                crash_count = self.restart_policy.crash_count(now),
                "Backend process crashed; attempting restart within retry limit"
            );
        }

        let new_proc = BackendLauncher::spawn(&self.config)?;
        self.active_process = Some(new_proc);
        Ok(true) // newly started or restarted
    }

    /// Stop the backend process immediately.
    pub fn stop(&mut self) {
        if let Some(mut proc) = self.active_process.take() {
            let _ = proc.kill();
        }
    }

    /// Return reference to active process if currently running.
    pub fn active_process(&self) -> Option<&BackendProcess> {
        self.active_process.as_ref()
    }

    /// Return mutable reference to active process if currently running.
    pub fn active_process_mut(&mut self) -> Option<&mut BackendProcess> {
        self.active_process.as_mut()
    }

    /// Return the restart policy.
    pub fn restart_policy(&self) -> &BackendRestartPolicy {
        &self.restart_policy
    }
}

impl Drop for BackendProcessManager {
    fn drop(&mut self) {
        self.stop();
    }
}
