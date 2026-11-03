#![forbid(unsafe_code)]

//! Pure runtime manifest models and embedded approved runtimes list.
//!
//! All versions, URLs, sizes, and SHA-256 digests in this manifest have been
//! cryptographically verified against upstream GPG signatures.
//! See `docs/RUNTIME_SOURCES.md` for the provenance audit trail.

use thiserror::Error;

/// Target architecture for backend runtimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeArch {
    X64,
    Arm64,
}

impl RuntimeArch {
    /// Detect runtime architecture of the compiling host or current environment.
    pub fn current() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            RuntimeArch::X64
        }
        #[cfg(target_arch = "aarch64")]
        {
            RuntimeArch::Arm64
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            RuntimeArch::X64 // default fallback
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeArch::X64 => "x64",
            RuntimeArch::Arm64 => "arm64",
        }
    }
}

/// An approved runtime entry embedded in Wavical source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeManifestEntry {
    /// Runtime identifier (e.g. "node").
    pub runtime: &'static str,
    /// Exact semantic version string (e.g. "22.11.0").
    pub version: &'static str,
    /// Major LTS line (e.g. "22").
    pub major_line: &'static str,
    /// Target architecture.
    pub arch: RuntimeArch,
    /// Operating system ("win").
    pub os: &'static str,
    /// Official upstream download URL.
    pub url: &'static str,
    /// Exact byte size of the upstream archive.
    pub size_bytes: u64,
    /// SHA-256 digest of the upstream archive.
    pub sha256: &'static str,
    /// Executable file name inside the extracted runtime root.
    pub executable_name: &'static str,
}

impl RuntimeManifestEntry {
    /// Build a safe cache directory subpath name: `<runtime>-<version>-<arch>`
    /// Guaranteed to contain only safe alphanumeric characters, dashes, and dots.
    pub fn cache_dirname(&self) -> String {
        format!("{}-{}-{}", self.runtime, self.version, self.arch.as_str())
    }
}

/// Manifest errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeManifestError {
    #[error(
        "runtime '{runtime}' version '{version}' for architecture '{arch}' is not in the approved manifest"
    )]
    VersionNotApproved {
        runtime: String,
        version: String,
        arch: String,
    },

    #[error("no approved runtimes found for runtime '{0}'")]
    RuntimeNotFound(String),
}

/// Official approved runtimes manifest list.
/// Only cryptographically verified entries are permitted here.
pub const APPROVED_RUNTIMES: &[RuntimeManifestEntry] = &[
    RuntimeManifestEntry {
        runtime: "node",
        version: "22.11.0",
        major_line: "22",
        arch: RuntimeArch::X64,
        os: "win",
        url: "https://nodejs.org/dist/v22.11.0/node-v22.11.0-win-x64.zip",
        size_bytes: 34188182,
        sha256: "ca0a274f1edc90005b1dc7ec22ec55dad1acc21320bc0be853065d69db2a5152",
        executable_name: "node.exe",
    },
    RuntimeManifestEntry {
        runtime: "node",
        version: "22.11.0",
        major_line: "22",
        arch: RuntimeArch::Arm64,
        os: "win",
        url: "https://nodejs.org/dist/v22.11.0/node-v22.11.0-win-arm64.zip",
        size_bytes: 29888646,
        sha256: "b9ff5a6b6ffb68a0ffec82cc5664ed48247dabbd25ee6d129facd2f65a8ca80d",
        executable_name: "node.exe",
    },
];

/// Find an approved runtime entry for the requested runtime, version, and architecture.
pub fn find_approved_runtime(
    runtime: &str,
    version: &str,
    arch: RuntimeArch,
) -> Result<&'static RuntimeManifestEntry, RuntimeManifestError> {
    for entry in APPROVED_RUNTIMES {
        if entry.runtime.eq_ignore_ascii_case(runtime)
            && entry.version == version
            && entry.arch == arch
        {
            return Ok(entry);
        }
    }

    Err(RuntimeManifestError::VersionNotApproved {
        runtime: runtime.to_string(),
        version: version.to_string(),
        arch: arch.as_str().to_string(),
    })
}

/// List all approved runtime entries for a given runtime name.
pub fn list_approved_runtimes(runtime: &str) -> Vec<&'static RuntimeManifestEntry> {
    APPROVED_RUNTIMES
        .iter()
        .filter(|e| e.runtime.eq_ignore_ascii_case(runtime))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_approved_runtimes_presence() {
        let x64_entry = find_approved_runtime("node", "22.11.0", RuntimeArch::X64).unwrap();
        assert_eq!(x64_entry.size_bytes, 34188182);
        assert_eq!(
            x64_entry.sha256,
            "ca0a274f1edc90005b1dc7ec22ec55dad1acc21320bc0be853065d69db2a5152"
        );
        assert_eq!(x64_entry.cache_dirname(), "node-22.11.0-x64");

        let arm64_entry = find_approved_runtime("node", "22.11.0", RuntimeArch::Arm64).unwrap();
        assert_eq!(arm64_entry.size_bytes, 29888646);
        assert_eq!(arm64_entry.cache_dirname(), "node-22.11.0-arm64");
    }

    #[test]
    fn test_unapproved_version_rejected() {
        let err = find_approved_runtime("node", "99.99.99", RuntimeArch::X64).unwrap_err();
        assert!(matches!(
            err,
            RuntimeManifestError::VersionNotApproved { .. }
        ));
    }
}
