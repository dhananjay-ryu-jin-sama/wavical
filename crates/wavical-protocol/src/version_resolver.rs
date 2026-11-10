#![forbid(unsafe_code)]

//! Pure version resolution logic for Wavical backend runtimes.
//!
//! Evaluates pinned versions against approved runtime manifests according
//! to the approved version policy (WP1 Decision 2: Option A, same major LTS line).

use std::fmt;
use thiserror::Error;

/// Semantic version components: major.minor.patch
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemverVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl SemverVersion {
    /// Parse a semantic version string in exact "major.minor.patch" format.
    /// Rejects empty components, non-digit characters, leading plus/v, etc.
    pub fn parse(s: &str) -> Result<Self, VersionResolverError> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(VersionResolverError::InvalidVersionFormat(s.to_string()));
        }

        let parse_part = |p: &str| -> Result<u64, VersionResolverError> {
            if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
                return Err(VersionResolverError::InvalidVersionFormat(s.to_string()));
            }
            if p.len() > 1 && p.starts_with('0') {
                return Err(VersionResolverError::InvalidVersionFormat(s.to_string()));
            }
            p.parse::<u64>()
                .map_err(|_| VersionResolverError::InvalidVersionFormat(s.to_string()))
        };

        Ok(SemverVersion {
            major: parse_part(parts[0])?,
            minor: parse_part(parts[1])?,
            patch: parse_part(parts[2])?,
        })
    }
}

impl fmt::Display for SemverVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Version resolution policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionPolicy {
    /// WP1 Approved Decision: Same major LTS line (e.g. 22.x.y for Node 22).
    SameMajorLts,
    /// Same major.minor line (e.g. 22.11.x).
    SameMajorMinor,
}

/// Version resolution errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VersionResolverError {
    #[error("invalid semver version format '{0}'; must be exact 'major.minor.patch'")]
    InvalidVersionFormat(String),

    #[error("pinned version '{0}' is not present in approved runtime manifest")]
    VersionNotFoundInManifest(String),

    #[error("no approved versions match policy for pinned version '{0}'")]
    NoMatchingVersion(String),
}

/// Resolve a pinned version against a list of available manifest versions using the default approved policy (SameMajorLts).
pub fn resolve_runtime_version(
    pinned: &str,
    allow_patch_updates: bool,
    available_versions: &[&str],
) -> Result<String, VersionResolverError> {
    resolve_runtime_version_with_policy(
        pinned,
        allow_patch_updates,
        available_versions,
        VersionPolicy::SameMajorLts,
    )
}

/// Resolve a pinned version against available manifest versions with an explicit policy.
pub fn resolve_runtime_version_with_policy(
    pinned: &str,
    allow_patch_updates: bool,
    available_versions: &[&str],
    policy: VersionPolicy,
) -> Result<String, VersionResolverError> {
    let pinned_semver = SemverVersion::parse(pinned)?;

    let pinned_in_manifest = available_versions.contains(&pinned);

    if !allow_patch_updates {
        if pinned_in_manifest {
            return Ok(pinned.to_string());
        } else {
            return Err(VersionResolverError::VersionNotFoundInManifest(
                pinned.to_string(),
            ));
        }
    }

    let mut matching_candidates: Vec<(SemverVersion, &str)> = Vec::new();

    for &cand_str in available_versions {
        if let Ok(cand_semver) = SemverVersion::parse(cand_str) {
            let matches_policy = match policy {
                VersionPolicy::SameMajorLts => {
                    cand_semver.major == pinned_semver.major && cand_semver >= pinned_semver
                }
                VersionPolicy::SameMajorMinor => {
                    cand_semver.major == pinned_semver.major
                        && cand_semver.minor == pinned_semver.minor
                        && cand_semver >= pinned_semver
                }
            };
            if matches_policy {
                matching_candidates.push((cand_semver, cand_str));
            }
        }
    }

    if let Some((_, best_str)) = matching_candidates.into_iter().max_by_key(|(s, _)| *s) {
        Ok(best_str.to_string())
    } else if pinned_in_manifest {
        Ok(pinned.to_string())
    } else {
        Err(VersionResolverError::VersionNotFoundInManifest(
            pinned.to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_semver_parse_valid() {
        let v = SemverVersion::parse("22.11.0").unwrap();
        assert_eq!(
            v,
            SemverVersion {
                major: 22,
                minor: 11,
                patch: 0
            }
        );
        assert_eq!(v.to_string(), "22.11.0");
    }

    #[test]
    fn test_semver_parse_invalid() {
        assert!(SemverVersion::parse("22.11").is_err());
        assert!(SemverVersion::parse("v22.11.0").is_err());
        assert!(SemverVersion::parse("22.11.0-alpha").is_err());
        assert!(SemverVersion::parse("022.11.0").is_err());
        assert!(SemverVersion::parse("").is_err());
        assert!(SemverVersion::parse("abc").is_err());
    }

    #[test]
    fn test_resolve_pinned_when_patch_updates_disabled() {
        let manifest = ["22.11.0", "22.12.0", "22.13.0"];
        let res = resolve_runtime_version("22.11.0", false, &manifest).unwrap();
        assert_eq!(res, "22.11.0");

        let err = resolve_runtime_version("20.10.0", false, &manifest).unwrap_err();
        assert!(matches!(
            err,
            VersionResolverError::VersionNotFoundInManifest(_)
        ));
    }

    #[test]
    fn test_resolve_same_major_lts_upgrades_to_highest_in_line() {
        let manifest = ["20.18.0", "22.11.0", "22.12.0", "22.13.1", "24.0.0"];
        let res = resolve_runtime_version("22.11.0", true, &manifest).unwrap();
        assert_eq!(res, "22.13.1");

        // If pinned is 22.12.0, should resolve to 22.13.1
        let res2 = resolve_runtime_version("22.12.0", true, &manifest).unwrap();
        assert_eq!(res2, "22.13.1");

        // Should never jump to major 24
        assert_ne!(res, "24.0.0");
    }

    #[test]
    fn test_resolve_same_major_minor_policy() {
        let manifest = ["22.11.0", "22.11.1", "22.11.2", "22.12.0"];
        let res = resolve_runtime_version_with_policy(
            "22.11.0",
            true,
            &manifest,
            VersionPolicy::SameMajorMinor,
        )
        .unwrap();
        assert_eq!(res, "22.11.2");
    }

    // Property-based test suite (10,000 cases)
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10_000))]

        #[test]
        fn prop_version_resolver_invariants(
            major in 0u64..100,
            minor in 0u64..100,
            patch in 0u64..100,
            allow_patch in any::<bool>(),
            policy_is_major_lts in any::<bool>(),
            extra_cand_minors in proptest::collection::vec(0u64..100, 0..5),
            extra_cand_patches in proptest::collection::vec(0u64..100, 0..5),
            include_pin in any::<bool>(),
        ) {
            let pinned = format!("{major}.{minor}.{patch}");
            let policy = if policy_is_major_lts {
                VersionPolicy::SameMajorLts
            } else {
                VersionPolicy::SameMajorMinor
            };

            // Build manifest strings
            let mut manifest_strings = Vec::new();
            if include_pin {
                manifest_strings.push(pinned.clone());
            }
            for m in &extra_cand_minors {
                for p in &extra_cand_patches {
                    manifest_strings.push(format!("{major}.{m}.{p}"));
                }
            }
            // Add some other majors
            manifest_strings.push(format!("{}.0.0", major + 1));
            if major > 0 {
                manifest_strings.push(format!("{}.0.0", major - 1));
            }

            let manifest_refs: Vec<&str> = manifest_strings.iter().map(|s| s.as_str()).collect();

            let res1 = resolve_runtime_version_with_policy(&pinned, allow_patch, &manifest_refs, policy);
            let res2 = resolve_runtime_version_with_policy(&pinned, allow_patch, &manifest_refs, policy);

            // Invariant 1: Deterministic
            prop_assert_eq!(&res1, &res2);

            if let Ok(ref resolved) = res1 {
                let resolved_semver = SemverVersion::parse(resolved).expect("resolved must be valid semver");

                // Invariant 2: Never returns a version absent from the manifest
                prop_assert!(manifest_refs.contains(&resolved.as_str()));

                // Invariant 3: Never returns a different major
                prop_assert_eq!(resolved_semver.major, major);

                // Invariant 4: Never returns a different minor if policy is SameMajorMinor
                if policy == VersionPolicy::SameMajorMinor {
                    prop_assert_eq!(resolved_semver.minor, minor);
                }

                // Invariant 5: When patch updates are off, returns exactly the pin
                if !allow_patch {
                    prop_assert_eq!(resolved, &pinned);
                }

                // Invariant 6: Resolved version is >= pinned
                let pinned_semver = SemverVersion::parse(&pinned).expect("pinned must be valid semver");
                prop_assert!(resolved_semver >= pinned_semver);
            }
        }
    }
}
