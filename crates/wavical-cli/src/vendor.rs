//! `wavical vendor` — offline vendoring of pinned CDN assets (Plan 1.11, WP7).

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use url::{Host, Url};
use wavical_ai::downloader::{DownloadKind, DownloadOptions, download_bounded_bytes};
use wavical_netpolicy::limits::{
    MAX_VENDOR_FILE_BYTES, MAX_VENDOR_FILE_COUNT, MAX_VENDOR_TOTAL_BYTES,
};
use wavical_netpolicy::vendor_lock::{VendorLock, VendorLockEntry, is_safe_vendor_local_path};
use wavical_netpolicy::version::{VersionPinStatus, detect_url_version_pin};

use crate::CliError;
use crate::html_scan::{scan_css_refs, scan_html};

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "CONIN$",
    "CONOUT$",
];

/// Options controlling `wavical vendor`.
#[derive(Debug, Clone, Default)]
pub struct VendorRunOptions {
    pub allow_unpinned: Vec<String>,
    pub check: bool,
    pub update: bool,
    pub download_options: DownloadOptions,
}

/// Converts a validated `https://` (or test-localhost `http://`) URL into a safe relative path
/// strictly under `vendor/<host>/<normalized_path>`.
pub fn url_to_vendor_rel_path(raw_url: &str, default_ext: &str) -> Result<String, CliError> {
    let parsed = Url::parse(raw_url.trim())
        .map_err(|_| CliError::ShellError(format!("invalid vendor URL: {raw_url}")))?;
    let Some(Host::Domain(host_raw)) = parsed.host() else {
        // Allow 127.0.0.1 in tests
        let host_str = parsed
            .host_str()
            .ok_or_else(|| CliError::ShellError("vendor URL missing host".into()))?;
        return build_vendor_path(host_str, parsed.path(), parsed.query(), default_ext);
    };
    build_vendor_path(
        &host_raw.to_ascii_lowercase(),
        parsed.path(),
        parsed.query(),
        default_ext,
    )
}

fn build_vendor_path(
    host: &str,
    url_path: &str,
    query: Option<&str>,
    default_ext: &str,
) -> Result<String, CliError> {
    let clean_host = host.replace(':', "_").to_ascii_lowercase();
    if clean_host.is_empty() || clean_host.contains("..") || clean_host.contains('/') {
        return Err(CliError::ShellError("unsafe host in vendor URL".into()));
    }

    let mut segs: Vec<String> = Vec::new();
    for raw_seg in url_path.split('/').filter(|s| !s.is_empty()) {
        if raw_seg == "." || raw_seg == ".." || raw_seg.contains('\\') || raw_seg.contains(':') {
            return Err(CliError::ShellError(format!(
                "unsafe path segment '{raw_seg}' in vendor URL"
            )));
        }
        let sanitized: String = raw_seg
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let trimmed = sanitized.trim_end_matches(['.', ' ']);
        if trimmed.is_empty() {
            return Err(CliError::ShellError("empty sanitized path segment".into()));
        }
        let stem = trimmed.split('.').next().unwrap_or(trimmed);
        if WINDOWS_RESERVED
            .iter()
            .any(|r| stem.eq_ignore_ascii_case(r))
        {
            return Err(CliError::ShellError(format!(
                "reserved Windows device name '{stem}' in vendor URL"
            )));
        }
        segs.push(trimmed.to_string());
    }

    if segs.is_empty() {
        segs.push(format!("index.{default_ext}"));
    }

    if let Some(q) = query
        && !q.is_empty()
        && let Some(last) = segs.last_mut()
    {
        let q_hash = hex::encode(&Sha256::digest(q.as_bytes())[..4]);
        if let Some((base, ext)) = last.rsplit_once('.') {
            *last = format!("{base}_{q_hash}.{ext}");
        } else {
            *last = format!("{last}_{q_hash}.{default_ext}");
        }
    } else if let Some(last) = segs.last_mut()
        && !last.contains('.')
    {
        last.push('.');
        last.push_str(default_ext);
    }

    let rel = format!("vendor/{clean_host}/{}", segs.join("/"));
    if rel.len() > 240 || !is_safe_vendor_local_path(&rel) {
        return Err(CliError::ShellError(format!(
            "unsafe or overly long vendor path '{rel}'"
        )));
    }
    Ok(rel)
}

/// Runs `wavical vendor` (or `--check` / `--update`).
pub fn run_vendor(project_root: &Path, opts: &VendorRunOptions) -> Result<(), CliError> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    run_vendor_with_io(project_root, opts, &mut reader, &mut writer)
}

/// Testable core of `wavical vendor`.
pub fn run_vendor_with_io<R: BufRead, W: Write>(
    project_root: &Path,
    opts: &VendorRunOptions,
    reader: &mut R,
    writer: &mut W,
) -> Result<(), CliError> {
    let src_dir = project_root.join("src");
    let lock_path = project_root.join("vendor.lock.json");

    if opts.check {
        return verify_vendor_lock(
            &src_dir,
            &lock_path,
            opts.download_options.allow_test_localhost,
            writer,
        );
    }
    if opts.update {
        return update_vendor_lock(&src_dir, &lock_path, opts, reader, writer);
    }

    if !src_dir.is_dir() {
        return Err(CliError::SrcNotFound(src_dir));
    }

    let allow_unpinned_set: HashSet<String> = opts
        .allow_unpinned
        .iter()
        .map(|s| s.trim().to_string())
        .collect();

    // 1. Collect all external URLs from HTML and CSS in src/
    let mut html_files = Vec::new();
    let mut css_files = Vec::new();
    collect_frontend_files(&src_dir, &mut html_files, &mut css_files)?;

    let mut target_urls: BTreeMap<String, &'static str> = BTreeMap::new();
    for html_path in &html_files {
        let content =
            fs::read_to_string(html_path).map_err(|e| CliError::ShellError(e.to_string()))?;
        let rel = html_path
            .strip_prefix(&src_dir)
            .unwrap_or(html_path)
            .to_string_lossy()
            .replace('\\', "/");
        let scan = scan_html(&rel, &content);
        for ext in scan.external_refs {
            let default_ext = match ext.category {
                wavical_netpolicy::html::ExternalRefCategory::Script => "js",
                wavical_netpolicy::html::ExternalRefCategory::Style => "css",
                _ => "bin",
            };
            target_urls.entry(ext.url).or_insert(default_ext);
        }
    }
    for css_path in &css_files {
        let content =
            fs::read_to_string(css_path).map_err(|e| CliError::ShellError(e.to_string()))?;
        for cref in scan_css_refs(&content) {
            if is_http_or_https(&cref.url) {
                let default_ext = if cref.is_import { "css" } else { "woff2" };
                target_urls.entry(cref.url).or_insert(default_ext);
            }
        }
    }

    // 2. Enforce pinned-version policy (Plan 1.11)
    for url in target_urls.keys() {
        if !opts.download_options.allow_test_localhost && !url.starts_with("https://") {
            return Err(CliError::ShellError(format!(
                "refusing non-HTTPS URL '{url}' in wavical vendor"
            )));
        }
        match detect_url_version_pin(url) {
            VersionPinStatus::Pinned(_) => {}
            VersionPinStatus::Unpinned(_) | VersionPinStatus::Unknown => {
                if allow_unpinned_set.contains(url) {
                    writeln!(
                        writer,
                        "WARNING: Taking one-time unpinned snapshot of '{url}'; it will never update by itself."
                    )
                    .map_err(|e| CliError::ShellError(e.to_string()))?;
                } else {
                    return Err(CliError::ShellError(format!(
                        "Refusing unpinned URL '{url}'. Please use a pinned version URL, or pass --allow-unpinned {url} for a one-time snapshot."
                    )));
                }
            }
        }
    }

    // 3. Download assets through Phase 4 shared verified downloader + follow CSS 1 level
    let mut lock_entries: Vec<VendorLockEntry> = Vec::new();
    let mut url_to_local: BTreeMap<String, String> = BTreeMap::new();
    let mut total_bytes: u64 = 0;

    for (url, default_ext) in &target_urls {
        let bytes = download_bounded_bytes(
            url,
            MAX_VENDOR_FILE_BYTES as usize,
            DownloadKind::Vendor,
            &opts.download_options,
        )
        .map_err(|e| CliError::ShellError(format!("download failed for '{url}': {e}")))?;

        let rel_local = url_to_vendor_rel_path(url, default_ext)?;
        let mut final_bytes = bytes;

        // If CSS, follow url() and @import one level
        if rel_local.ends_with(".css")
            && let Ok(css_text) = std::str::from_utf8(&final_bytes)
        {
            let base_url = Url::parse(url).map_err(|e| CliError::ShellError(e.to_string()))?;
            let refs = scan_css_refs(css_text);
            let mut css_replacements: Vec<(usize, usize, String)> = Vec::new();

            for cref in refs {
                if cref.url.starts_with("data:") || cref.url.starts_with('#') {
                    continue;
                }
                if cref.url.contains("..") || cref.url.starts_with('/') {
                    // Absolute path or traversal in CSS: check if it escapes vendor root!
                    if cref.url.contains("..") {
                        return Err(CliError::ShellError(format!(
                            "CSS reference '{}' attempts path traversal outside vendor/",
                            cref.url
                        )));
                    }
                }
                let resolved_url = base_url
                    .join(&cref.url)
                    .map_err(|_| CliError::ShellError(format!("invalid CSS ref '{}'", cref.url)))?;
                if resolved_url.scheme() != "https"
                    && !(opts.download_options.allow_test_localhost
                        && resolved_url.scheme() == "http")
                {
                    return Err(CliError::ShellError(format!(
                        "refusing non-HTTPS CSS subresource '{resolved_url}'"
                    )));
                }
                let sub_url = resolved_url.to_string();
                let sub_rel = if let Some(existing) = url_to_local.get(&sub_url) {
                    existing.clone()
                } else {
                    let sub_bytes = download_bounded_bytes(
                        &sub_url,
                        MAX_VENDOR_FILE_BYTES as usize,
                        DownloadKind::Vendor,
                        &opts.download_options,
                    )
                    .map_err(|e| {
                        CliError::ShellError(format!("download failed for '{sub_url}': {e}"))
                    })?;

                    let sr = url_to_vendor_rel_path(
                        &sub_url,
                        if cref.is_import { "css" } else { "woff2" },
                    )?;
                    record_and_write_vendor_file(
                        &src_dir,
                        &sub_url,
                        &sr,
                        &sub_bytes,
                        &mut total_bytes,
                        &mut lock_entries,
                    )?;
                    url_to_local.insert(sub_url, sr.clone());
                    sr
                };

                let rel_from_css = relative_path_between(&rel_local, &sub_rel);
                css_replacements.push((cref.start, cref.end, rel_from_css));
            }

            if !css_replacements.is_empty() {
                final_bytes = apply_byte_replacements(css_text, &css_replacements).into_bytes();
            }
        }

        record_and_write_vendor_file(
            &src_dir,
            url,
            &rel_local,
            &final_bytes,
            &mut total_bytes,
            &mut lock_entries,
        )?;
        url_to_local.insert(url.clone(), rel_local);
    }

    // 4. Write vendor.lock.json (validated with WP2 parser first!)
    let lock = VendorLock {
        version: 1,
        entries: lock_entries,
    };
    let lock_json =
        serde_json::to_string_pretty(&lock).map_err(|e| CliError::ShellError(e.to_string()))?;
    VendorLock::parse_bytes_with_options(
        lock_json.as_bytes(),
        opts.download_options.allow_test_localhost,
    )
    .map_err(|e| CliError::ShellError(format!("vendor.lock.json validation failed: {e}")))?;
    fs::write(&lock_path, lock_json).map_err(|e| CliError::WriteFileFailed {
        path: lock_path,
        source: e,
    })?;

    // 5. Rewrite HTML & CSS references in src/ with backups and diff summary
    for html_path in &html_files {
        let original =
            fs::read_to_string(html_path).map_err(|e| CliError::ShellError(e.to_string()))?;
        let mut updated = original.clone();
        let mut changed_count = 0usize;
        for (url, local_rel) in &url_to_local {
            if updated.contains(url.as_str()) {
                updated = updated.replace(url.as_str(), local_rel);
                changed_count += 1;
            }
        }
        if updated != original {
            let bak_path = PathBuf::from(format!("{}.bak", html_path.display()));
            fs::write(&bak_path, &original).map_err(|e| CliError::WriteFileFailed {
                path: bak_path,
                source: e,
            })?;
            fs::write(html_path, &updated).map_err(|e| CliError::WriteFileFailed {
                path: html_path.clone(),
                source: e,
            })?;
            writeln!(
                writer,
                "Rewrote {} reference(s) in {} (backup saved)",
                changed_count,
                html_path.display()
            )
            .map_err(|e| CliError::ShellError(e.to_string()))?;
        }
    }

    // 6. Print licenses & mandatory notice
    writeln!(writer, "\nVendored {} file(s):", lock.entries.len())
        .map_err(|e| CliError::ShellError(e.to_string()))?;
    for entry in &lock.entries {
        let lic = entry.license.as_deref().unwrap_or("unspecified");
        writeln!(writer, "  - {} [license: {lic}]", entry.local_path)
            .map_err(|e| CliError::ShellError(e.to_string()))?;
    }
    writeln!(
        writer,
        "Vendoring redistributes these files inside your app. You are responsible for their licenses."
    )
    .map_err(|e| CliError::ShellError(e.to_string()))?;

    Ok(())
}

fn record_and_write_vendor_file(
    src_dir: &Path,
    url: &str,
    rel_local: &str,
    bytes: &[u8],
    total_bytes: &mut u64,
    lock_entries: &mut Vec<VendorLockEntry>,
) -> Result<(), CliError> {
    if lock_entries.len() >= MAX_VENDOR_FILE_COUNT {
        return Err(CliError::ShellError(format!(
            "vendored file count exceeds limit of {MAX_VENDOR_FILE_COUNT}"
        )));
    }
    let size = bytes.len() as u64;
    if size > MAX_VENDOR_FILE_BYTES {
        return Err(CliError::ShellError(format!(
            "vendored file '{rel_local}' ({size} bytes) exceeds max of {MAX_VENDOR_FILE_BYTES} bytes"
        )));
    }
    *total_bytes = total_bytes
        .checked_add(size)
        .ok_or_else(|| CliError::ShellError("total vendor size overflow".into()))?;
    if *total_bytes > MAX_VENDOR_TOTAL_BYTES {
        return Err(CliError::ShellError(format!(
            "total vendored size exceeds max of {MAX_VENDOR_TOTAL_BYTES} bytes"
        )));
    }

    if !is_safe_vendor_local_path(rel_local) {
        return Err(CliError::ShellError(format!(
            "refusing unsafe vendor path '{rel_local}'"
        )));
    }

    let dst = src_dir.join(rel_local);
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| CliError::CreateDirFailed {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    fs::write(&dst, bytes).map_err(|e| CliError::WriteFileFailed {
        path: dst,
        source: e,
    })?;

    let sha256 = hex::encode(Sha256::digest(bytes));
    let license = detect_license_header(bytes);
    lock_entries.push(VendorLockEntry {
        url: url.to_string(),
        local_path: rel_local.to_string(),
        sha256,
        size,
        fetched_at: "2026-11-21T00:00:00Z".to_string(),
        license,
    });
    Ok(())
}

fn verify_vendor_lock<W: Write>(
    src_dir: &Path,
    lock_path: &Path,
    allow_test_localhost: bool,
    writer: &mut W,
) -> Result<(), CliError> {
    if !lock_path.is_file() {
        return Err(CliError::ShellError("vendor.lock.json not found".into()));
    }
    let lock_bytes = fs::read(lock_path).map_err(|e| CliError::ShellError(e.to_string()))?;
    let lock = VendorLock::parse_bytes_with_options(&lock_bytes, allow_test_localhost)
        .map_err(|e| CliError::ShellError(format!("invalid vendor.lock.json: {e}")))?;

    for entry in &lock.entries {
        let p = src_dir.join(&entry.local_path);
        if !p.is_file() {
            return Err(CliError::ShellError(format!(
                "vendored file missing: {}",
                entry.local_path
            )));
        }
        let bytes = fs::read(&p).map_err(|e| CliError::ShellError(e.to_string()))?;
        let actual_sha = hex::encode(Sha256::digest(&bytes));
        if bytes.len() as u64 != entry.size || actual_sha != entry.sha256 {
            return Err(CliError::ShellError(format!(
                "vendor integrity check failed for '{}': expected {} ({} bytes), got {} ({} bytes)",
                entry.local_path,
                entry.sha256,
                entry.size,
                actual_sha,
                bytes.len()
            )));
        }
    }
    writeln!(
        writer,
        "✓ Verified {} vendored file(s).",
        lock.entries.len()
    )
    .map_err(|e| CliError::ShellError(e.to_string()))?;
    Ok(())
}

fn update_vendor_lock<R: BufRead, W: Write>(
    src_dir: &Path,
    lock_path: &Path,
    opts: &VendorRunOptions,
    reader: &mut R,
    writer: &mut W,
) -> Result<(), CliError> {
    if !lock_path.is_file() {
        return Err(CliError::ShellError("vendor.lock.json not found".into()));
    }
    let lock_bytes = fs::read(lock_path).map_err(|e| CliError::ShellError(e.to_string()))?;
    let mut lock = VendorLock::parse_bytes(&lock_bytes)
        .map_err(|e| CliError::ShellError(format!("invalid vendor.lock.json: {e}")))?;

    let mut updates: Vec<(usize, Vec<u8>, String)> = Vec::new();
    for (idx, entry) in lock.entries.iter().enumerate() {
        if !matches!(
            detect_url_version_pin(&entry.url),
            VersionPinStatus::Pinned(_)
        ) {
            continue;
        }
        let new_bytes = download_bounded_bytes(
            &entry.url,
            MAX_VENDOR_FILE_BYTES as usize,
            DownloadKind::Vendor,
            &opts.download_options,
        )
        .map_err(|e| CliError::ShellError(format!("update failed for '{}': {e}", entry.url)))?;
        let new_sha = hex::encode(Sha256::digest(&new_bytes));
        if new_sha != entry.sha256 {
            writeln!(
                writer,
                "Changed: {} ({} -> {})",
                entry.local_path, entry.sha256, new_sha
            )
            .map_err(|e| CliError::ShellError(e.to_string()))?;
            updates.push((idx, new_bytes, new_sha));
        }
    }

    if updates.is_empty() {
        writeln!(writer, "All pinned vendored files are up to date.")
            .map_err(|e| CliError::ShellError(e.to_string()))?;
        return Ok(());
    }

    write!(writer, "Apply {} update(s)? [y/N]: ", updates.len())
        .map_err(|e| CliError::ShellError(e.to_string()))?;
    writer
        .flush()
        .map_err(|e| CliError::ShellError(e.to_string()))?;
    let mut ans = String::new();
    reader
        .read_line(&mut ans)
        .map_err(|e| CliError::ShellError(e.to_string()))?;
    if !matches!(ans.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        writeln!(writer, "Update cancelled.").map_err(|e| CliError::ShellError(e.to_string()))?;
        return Ok(());
    }

    for (idx, bytes, sha) in updates {
        let entry = &mut lock.entries[idx];
        let dst = src_dir.join(&entry.local_path);
        fs::write(&dst, &bytes).map_err(|e| CliError::WriteFileFailed {
            path: dst,
            source: e,
        })?;
        entry.size = bytes.len() as u64;
        entry.sha256 = sha;
    }

    let out_json =
        serde_json::to_string_pretty(&lock).map_err(|e| CliError::ShellError(e.to_string()))?;
    fs::write(lock_path, out_json).map_err(|e| CliError::WriteFileFailed {
        path: lock_path.to_path_buf(),
        source: e,
    })?;
    Ok(())
}

fn detect_license_header(bytes: &[u8]) -> Option<String> {
    let head = std::str::from_utf8(&bytes[..bytes.len().min(1024)]).ok()?;
    for line in head.lines().take(20) {
        if let Some((_, rest)) = line.split_once("SPDX-License-Identifier:") {
            let lic = rest.trim().trim_end_matches("*/").trim();
            if !lic.is_empty() {
                return Some(lic.to_string());
            }
        }
    }
    None
}

fn is_http_or_https(raw: &str) -> bool {
    url::Url::parse(raw.trim()).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

fn apply_byte_replacements(src: &str, replacements: &[(usize, usize, String)]) -> String {
    let mut sorted = replacements.to_vec();
    sorted.sort_by_key(|r| r.0);
    let mut out = String::with_capacity(src.len());
    let mut cursor = 0usize;
    for (start, end, rep) in sorted {
        if start >= cursor && end <= src.len() {
            out.push_str(&src[cursor..start]);
            out.push_str(&rep);
            cursor = end;
        }
    }
    out.push_str(&src[cursor..]);
    out
}

fn relative_path_between(from_rel: &str, to_rel: &str) -> String {
    let from_parts: Vec<&str> = from_rel.split('/').collect();
    let to_parts: Vec<&str> = to_rel.split('/').collect();
    let from_dir_len = from_parts.len().saturating_sub(1);
    let mut common = 0usize;
    while common < from_dir_len && common < to_parts.len() && from_parts[common] == to_parts[common]
    {
        common += 1;
    }
    let mut out = vec![".."; from_dir_len.saturating_sub(common)];
    for &p in &to_parts[common..] {
        out.push(p);
    }
    out.join("/")
}

fn collect_frontend_files(
    dir: &Path,
    html_out: &mut Vec<PathBuf>,
    css_out: &mut Vec<PathBuf>,
) -> Result<(), CliError> {
    let entries = fs::read_dir(dir).map_err(|e| CliError::ShellError(e.to_string()))?;
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "vendor" || name == "node_modules" {
            continue;
        }
        if p.is_dir() {
            collect_frontend_files(&p, html_out, css_out)?;
        } else if p.is_file()
            && let Some(ext) = p.extension().and_then(|e| e.to_str())
        {
            match ext.to_ascii_lowercase().as_str() {
                "html" | "htm" => html_out.push(p),
                "css" => css_out.push(p),
                _ => {}
            }
        }
    }
    Ok(())
}
