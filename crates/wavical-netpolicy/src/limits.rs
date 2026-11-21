//! Named security and resource limits from Phase 4.5 Plan section 1.9.

/// Maximum size of `wavical.allowlist.json` in bytes (256 KB).
pub const MAX_ALLOWLIST_FILE_BYTES: usize = 256 * 1024;

/// Maximum entries per allowlist category list.
pub const MAX_ENTRIES_PER_LIST: usize = 200;

/// Maximum `localExceptions` entries in `wavical.allowlist.json`.
pub const MAX_LOCAL_EXCEPTIONS: usize = 20;

/// Maximum character length of an origin string.
pub const MAX_ORIGIN_LEN: usize = 2_048;

/// Maximum character length of a DNS host name.
pub const MAX_HOST_LEN: usize = 253;

/// Maximum character length of an `openExternal` URL.
pub const MAX_OPEN_EXTERNAL_URL_LEN: usize = 2_048;

/// Maximum `openExternal` calls per window within `OPEN_EXTERNAL_RATE_LIMIT_WINDOW_SECS`.
pub const OPEN_EXTERNAL_RATE_LIMIT_COUNT: usize = 5;

/// Sliding window duration in seconds for `openExternal` rate limiting.
pub const OPEN_EXTERNAL_RATE_LIMIT_WINDOW_SECS: u64 = 10;

/// Maximum recorded blocked events per `--record` session.
pub const MAX_RECORDED_EVENTS_PER_SESSION: usize = 10_000;

/// Maximum inline `<script>` blocks hashed per HTML page in `safe` mode.
pub const MAX_INLINE_SCRIPTS_HASHED_PER_PAGE: usize = 100;

/// Maximum size of a single vendored file in bytes (20 MB).
pub const MAX_VENDOR_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// Maximum total size of all vendored files in bytes (200 MB).
pub const MAX_VENDOR_TOTAL_BYTES: u64 = 200 * 1024 * 1024;

/// Maximum number of vendored files in `vendor.lock.json`.
pub const MAX_VENDOR_FILE_COUNT: usize = 500;
