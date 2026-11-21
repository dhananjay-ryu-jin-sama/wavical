//! CDN URL version pinning detection per WP1 decision table.

use url::{Host, Url};

/// Whether an external CDN URL has a pinned immutable version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionPinStatus {
    Pinned(String),
    Unpinned(String),
    Unknown,
}

/// Classifies an external resource URL as `Pinned`, `Unpinned`, or `Unknown`.
#[must_use]
pub fn detect_url_version_pin(raw_url: &str) -> VersionPinStatus {
    let Ok(parsed) = Url::parse(raw_url.trim()) else {
        return VersionPinStatus::Unknown;
    };
    let path = parsed.path();
    let Some(Host::Domain(host_raw)) = parsed.host() else {
        return detect_generic(path);
    };
    let host = host_raw.to_ascii_lowercase();

    if host == "cdnjs.cloudflare.com" {
        return detect_cdnjs(path);
    }
    if host == "cdn.jsdelivr.net" {
        return detect_jsdelivr(path);
    }
    if host == "unpkg.com" {
        return detect_unpkg(path);
    }
    if host == "fonts.googleapis.com" {
        return VersionPinStatus::Unpinned("dynamic-css-api".to_string());
    }
    if host == "fonts.gstatic.com" {
        return detect_gstatic(path);
    }
    if host == "cdn.tailwindcss.com" {
        return detect_tailwind(raw_url.trim(), path);
    }

    detect_generic(path)
}

fn detect_cdnjs(path: &str) -> VersionPinStatus {
    // /ajax/libs/<pkg>/<version>/<file>
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.len() >= 5 && segs[0] == "ajax" && segs[1] == "libs" {
        let ver = segs[3];
        if is_exact_semver(ver) {
            return VersionPinStatus::Pinned(ver.to_string());
        }
        return VersionPinStatus::Unpinned(ver.to_string());
    }
    VersionPinStatus::Unknown
}

fn detect_jsdelivr(path: &str) -> VersionPinStatus {
    // /npm/<pkg>@<ver>/<file> or /npm/@scope/<pkg>@<ver>/<file> or /gh/<user>/<repo>@<ver>/<file>
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return VersionPinStatus::Unpinned("root".to_string());
    }
    if segs[0] == "npm" && segs.len() >= 2 {
        let pkg_seg = if segs[1].starts_with('@') && segs.len() >= 3 {
            segs[2]
        } else {
            segs[1]
        };
        return classify_at_version(pkg_seg);
    }
    if segs[0] == "gh" && segs.len() >= 3 {
        return classify_at_version(segs[2]);
    }
    VersionPinStatus::Unknown
}

fn detect_unpkg(path: &str) -> VersionPinStatus {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return VersionPinStatus::Unpinned("root".to_string());
    }
    let pkg_seg = if segs[0].starts_with('@') && segs.len() >= 2 {
        segs[1]
    } else {
        segs[0]
    };
    classify_at_version(pkg_seg)
}

fn detect_gstatic(path: &str) -> VersionPinStatus {
    // /s/<font>/v30/...
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        if let Some(rest) = seg.strip_prefix('v')
            && !rest.is_empty()
            && rest.chars().all(|c| c.is_ascii_digit())
        {
            return VersionPinStatus::Pinned(seg.to_string());
        }
    }
    VersionPinStatus::Unknown
}

fn detect_tailwind(raw_url: &str, path: &str) -> VersionPinStatus {
    // https://cdn.tailwindcss.com/3.4.17 or https://cdn.tailwindcss.com@4.0.0
    if let Some((_, ver)) = raw_url.split_once('@') {
        let ver_clean = ver.split(['/', '?', '#']).next().unwrap_or("");
        if is_exact_semver(ver_clean) {
            return VersionPinStatus::Pinned(ver_clean.to_string());
        }
        return VersionPinStatus::Unpinned(ver_clean.to_string());
    }
    let seg = path.trim_start_matches('/').split('/').next().unwrap_or("");
    if seg.is_empty() {
        return VersionPinStatus::Unpinned("unversioned".to_string());
    }
    if is_exact_semver(seg) {
        return VersionPinStatus::Pinned(seg.to_string());
    }
    VersionPinStatus::Unpinned(seg.to_string())
}

fn detect_generic(path: &str) -> VersionPinStatus {
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        if is_exact_semver(seg) || is_full_commit_sha(seg) {
            return VersionPinStatus::Pinned(seg.to_string());
        }
        if matches!(
            seg,
            "latest" | "next" | "canary" | "nightly" | "main" | "master"
        ) {
            return VersionPinStatus::Unpinned(seg.to_string());
        }
    }
    VersionPinStatus::Unknown
}

fn classify_at_version(pkg_seg: &str) -> VersionPinStatus {
    let Some((_, ver)) = pkg_seg.rsplit_once('@') else {
        return VersionPinStatus::Unpinned("no-version".to_string());
    };
    if is_exact_semver(ver) || is_full_commit_sha(ver) {
        VersionPinStatus::Pinned(ver.to_string())
    } else {
        VersionPinStatus::Unpinned(ver.to_string())
    }
}

/// Returns true for `v?MAJOR.MINOR.PATCH(-prerelease)?` with numeric components.
#[must_use]
pub fn is_exact_semver(s: &str) -> bool {
    let s = s.strip_prefix('v').unwrap_or(s);
    let core = s.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn is_full_commit_sha(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}
