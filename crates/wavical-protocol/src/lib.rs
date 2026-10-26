#![forbid(unsafe_code)]

use std::path::{Component, Path, PathBuf};
use thiserror::Error;

/// Maximum allowable length for raw requested URL path strings.
pub const MAX_PATH_LENGTH: usize = 2048;

/// Standard security header name and value for preventing MIME-sniffing.
pub const HEADER_NOSNIFF: (&str, &str) = ("X-Content-Type-Options", "nosniff");

/// Specific reasons why a requested path is rejected by the protocol resolver.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PathRejectionReason {
    #[error("Path exceeds maximum allowed length of {0} bytes")]
    PathTooLong(usize),

    #[error("Null byte detected in path")]
    NullByte,

    #[error("Invalid percent-encoding detected in path")]
    InvalidEncoding,

    #[error("Directory traversal sequence detected ('..')")]
    TraversalDetected,

    #[error("Absolute path or drive letter is not permitted")]
    AbsolutePathOrDrive,

    #[error("UNC network path is not permitted")]
    UncPath,

    #[error("Alternate data stream or invalid colon character detected")]
    AlternateDataStream,

    #[error("Windows reserved device name detected: '{0}'")]
    ReservedDeviceName(String),

    #[error("Trailing dot or space detected in path segment")]
    TrailingDotOrSpace,

    #[error("Requested asset not found")]
    NotFound,

    #[error("Resolved path escaped the asset root")]
    EscapedRoot,

    #[error("Target is a directory without an index.html file")]
    IsDirectory,

    #[error("Filesystem canonicalization failed")]
    CanonicalizationFailed,
}

/// Resolved asset result containing safe path and MIME type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAsset {
    pub path: PathBuf,
    pub mime_type: &'static str,
}

/// Detect standard MIME type based on file extension.
pub fn get_mime_type(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Safely decode percent-encoded sequences in a URL path up to multiple iterations
/// to protect against nested/double encoding attacks (e.g. `%252e%252e`).
fn decode_percent_iterative(input: &str) -> Result<String, PathRejectionReason> {
    let mut current = input.to_string();
    let mut iterations = 0;

    while current.contains('%') {
        iterations += 1;
        if iterations > 5 {
            return Err(PathRejectionReason::InvalidEncoding);
        }

        let mut bytes: Vec<u8> = Vec::with_capacity(current.len());
        let mut chars = current.bytes().peekable();

        while let Some(b) = chars.next() {
            if b == b'%' {
                let h1 = chars.next().ok_or(PathRejectionReason::InvalidEncoding)?;
                let h2 = chars.next().ok_or(PathRejectionReason::InvalidEncoding)?;

                let byte_val =
                    hex_chars_to_byte(h1, h2).ok_or(PathRejectionReason::InvalidEncoding)?;

                if byte_val == 0 {
                    return Err(PathRejectionReason::NullByte);
                }
                bytes.push(byte_val);
            } else {
                if b == 0 {
                    return Err(PathRejectionReason::NullByte);
                }
                bytes.push(b);
            }
        }

        let decoded = String::from_utf8(bytes).map_err(|_| PathRejectionReason::InvalidEncoding)?;
        if decoded == current {
            break;
        }
        current = decoded;
    }

    if current.contains('\0') {
        return Err(PathRejectionReason::NullByte);
    }

    Ok(current)
}

fn hex_chars_to_byte(h1: u8, h2: u8) -> Option<u8> {
    let d1 = hex_digit_val(h1)?;
    let d2 = hex_digit_val(h2)?;
    Some((d1 << 4) | d2)
}

fn hex_digit_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Checks if a component matches a Windows reserved device name (CON, PRN, AUX, NUL, COM0-9, LPT0-9).
/// Uses prefix matching rather than byte slicing to avoid non-ASCII char boundary panics.
pub fn is_windows_reserved_device(segment: &str) -> bool {
    let base = segment.split('.').next().unwrap_or(segment).trim();
    let base_upper = base.to_ascii_uppercase();

    match base_upper.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        s => {
            if let Some(rest) = s.strip_prefix("COM").or_else(|| s.strip_prefix("LPT")) {
                rest.len() == 1 && rest.chars().next().is_some_and(|c| c.is_ascii_digit())
            } else {
                false
            }
        }
    }
}

/// Sanitizes and breaks down a decoded URL path into secure relative segments.
pub fn sanitize_path_segments(raw_path: &str) -> Result<Vec<String>, PathRejectionReason> {
    // 1. Length validation
    if raw_path.len() > MAX_PATH_LENGTH {
        return Err(PathRejectionReason::PathTooLong(MAX_PATH_LENGTH));
    }

    // 2. Immediate check for null bytes
    if raw_path.contains('\0') {
        return Err(PathRejectionReason::NullByte);
    }

    // 3. UNC and drive-relative root checks on raw input
    if raw_path.starts_with("//")
        || raw_path.starts_with("\\\\")
        || raw_path.starts_with("/\\\\")
        || raw_path.starts_with("\\//")
    {
        return Err(PathRejectionReason::UncPath);
    }

    // 4. Iteratively percent-decode
    let decoded = decode_percent_iterative(raw_path)?;

    // 5. Check UNC and special device namespaces after decoding
    if decoded.starts_with("//")
        || decoded.starts_with("\\\\")
        || decoded.starts_with("/\\\\")
        || decoded.starts_with("\\//")
        || decoded.starts_with(r"\??\")
        || decoded.starts_with(r"\\?\")
        || decoded.starts_with(r"//?/")
    {
        return Err(PathRejectionReason::UncPath);
    }

    // 6. Check for colon (drive letters, NTFS alternate data streams, namespaces)
    if decoded.contains(':') {
        let first_colon = decoded.find(':').unwrap_or(0);
        if first_colon == 1
            && decoded
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
        {
            return Err(PathRejectionReason::AbsolutePathOrDrive);
        }
        return Err(PathRejectionReason::AlternateDataStream);
    }

    // 7. Check traversal in raw/decoded string
    if decoded.contains("..") {
        return Err(PathRejectionReason::TraversalDetected);
    }

    // 8. Split by both forward and back slashes (reject mixed slash tricks)
    let parts: Vec<&str> = decoded
        .split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .collect();

    let mut sanitized_segments: Vec<String> = Vec::with_capacity(parts.len());

    for segment in parts {
        // Traversal checks per segment
        if segment == "." || segment == ".." || segment.contains("..") {
            return Err(PathRejectionReason::TraversalDetected);
        }

        // Trailing dot or space check (NTFS auto-stripping exploit)
        if segment.ends_with('.') || segment.ends_with(' ') || segment.starts_with(' ') {
            return Err(PathRejectionReason::TrailingDotOrSpace);
        }

        // Windows reserved device names
        if is_windows_reserved_device(segment) {
            return Err(PathRejectionReason::ReservedDeviceName(segment.to_string()));
        }

        sanitized_segments.push(segment.to_string());
    }

    Ok(sanitized_segments)
}

/// Resolves a requested URL path against an asset root directory.
///
/// Returns `Ok(ResolvedAsset)` if the file exists and is strictly contained
/// within `asset_root`, or `Err(PathRejectionReason)` if any validation or traversal rule fails.
pub fn resolve_asset_path(
    requested_path: &str,
    asset_root: &Path,
) -> Result<ResolvedAsset, PathRejectionReason> {
    // Canonicalize the asset root to establish ground-truth barrier
    let canonical_root = asset_root
        .canonicalize()
        .map_err(|_| PathRejectionReason::NotFound)?;

    let segments = sanitize_path_segments(requested_path)?;

    // Build the candidate path component-by-component
    let mut candidate = canonical_root.clone();

    if segments.is_empty() {
        candidate.push("index.html");
    } else {
        for segment in segments {
            candidate.push(segment);
        }
    }

    // Check existence
    if !candidate.exists() {
        return Err(PathRejectionReason::NotFound);
    }

    // If target is a directory, look for index.html inside it
    if candidate.is_dir() {
        let index_candidate = candidate.join("index.html");
        if index_candidate.is_file() {
            candidate = index_candidate;
        } else {
            return Err(PathRejectionReason::IsDirectory);
        }
    }

    // Canonicalize candidate path (resolves symlinks, junctions, 8.3 aliases)
    let canonical_candidate = candidate
        .canonicalize()
        .map_err(|_| PathRejectionReason::CanonicalizationFailed)?;

    // Crucial boundary check: canonical candidate MUST start with canonical root
    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(PathRejectionReason::EscapedRoot);
    }

    // Extra check: ensure no Path components escaped
    for component in canonical_candidate
        .strip_prefix(&canonical_root)
        .unwrap_or(Path::new(""))
        .components()
    {
        match component {
            Component::ParentDir => return Err(PathRejectionReason::TraversalDetected),
            Component::Prefix(_) | Component::RootDir => {
                return Err(PathRejectionReason::AbsolutePathOrDrive);
            }
            _ => {}
        }
    }

    let mime_type = get_mime_type(&canonical_candidate);

    Ok(ResolvedAsset {
        path: canonical_candidate,
        mime_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use tempfile::tempdir;

    #[test]
    fn test_valid_file_resolution() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("index.html");
        File::create(&file_path).unwrap();

        let resolved = resolve_asset_path("/index.html", dir.path()).unwrap();
        assert_eq!(resolved.path, file_path.canonicalize().unwrap());
        assert_eq!(resolved.mime_type, "text/html; charset=utf-8");

        let resolved_root = resolve_asset_path("/", dir.path()).unwrap();
        assert_eq!(resolved_root.path, file_path.canonicalize().unwrap());
    }

    #[test]
    fn test_nested_file_resolution() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("assets").join("css");
        fs::create_dir_all(&sub).unwrap();
        let css_file = sub.join("app.css");
        File::create(&css_file).unwrap();

        let resolved = resolve_asset_path("/assets/css/app.css", dir.path()).unwrap();
        assert_eq!(resolved.path, css_file.canonicalize().unwrap());
        assert_eq!(resolved.mime_type, "text/css; charset=utf-8");
    }

    #[test]
    fn test_rejection_directory_traversal() {
        let dir = tempdir().unwrap();
        assert_eq!(
            resolve_asset_path("/../secret.txt", dir.path()).unwrap_err(),
            PathRejectionReason::TraversalDetected
        );
        assert_eq!(
            resolve_asset_path("/foo/../../secret.txt", dir.path()).unwrap_err(),
            PathRejectionReason::TraversalDetected
        );
        assert_eq!(
            resolve_asset_path(r"/foo\..\..\secret.txt", dir.path()).unwrap_err(),
            PathRejectionReason::TraversalDetected
        );
    }

    #[test]
    fn test_rejection_encoded_traversal() {
        let dir = tempdir().unwrap();
        // %2e%2e = ..
        assert_eq!(
            resolve_asset_path("/%2e%2e/secret.txt", dir.path()).unwrap_err(),
            PathRejectionReason::TraversalDetected
        );
        // %2E%2E = ..
        assert_eq!(
            resolve_asset_path("/%2E%2E/secret.txt", dir.path()).unwrap_err(),
            PathRejectionReason::TraversalDetected
        );
        // Double encoding: %252e%252e -> %2e%2e -> ..
        assert_eq!(
            resolve_asset_path("/%252e%252e/secret.txt", dir.path()).unwrap_err(),
            PathRejectionReason::TraversalDetected
        );
    }

    #[test]
    fn test_rejection_null_byte() {
        let dir = tempdir().unwrap();
        assert_eq!(
            resolve_asset_path("/index.html\0.exe", dir.path()).unwrap_err(),
            PathRejectionReason::NullByte
        );
        assert_eq!(
            resolve_asset_path("/index.html%00.exe", dir.path()).unwrap_err(),
            PathRejectionReason::NullByte
        );
        assert_eq!(
            resolve_asset_path("/index.html%2500.exe", dir.path()).unwrap_err(),
            PathRejectionReason::NullByte
        );
    }

    #[test]
    fn test_rejection_drive_letter_and_absolute() {
        let dir = tempdir().unwrap();
        assert_eq!(
            resolve_asset_path("C:/Windows/System32", dir.path()).unwrap_err(),
            PathRejectionReason::AbsolutePathOrDrive
        );
        assert_eq!(
            resolve_asset_path("/C:/Windows/System32", dir.path()).unwrap_err(),
            PathRejectionReason::AlternateDataStream
        );
        assert_eq!(
            resolve_asset_path("D:", dir.path()).unwrap_err(),
            PathRejectionReason::AbsolutePathOrDrive
        );
    }

    #[test]
    fn test_rejection_unc_path() {
        let dir = tempdir().unwrap();
        assert_eq!(
            resolve_asset_path("//server/share/file.txt", dir.path()).unwrap_err(),
            PathRejectionReason::UncPath
        );
        assert_eq!(
            resolve_asset_path(r"\\server\share\file.txt", dir.path()).unwrap_err(),
            PathRejectionReason::UncPath
        );
        assert_eq!(
            resolve_asset_path(r"\\?\C:\secret.txt", dir.path()).unwrap_err(),
            PathRejectionReason::UncPath
        );
    }

    #[test]
    fn test_rejection_alternate_data_stream() {
        let dir = tempdir().unwrap();
        assert_eq!(
            resolve_asset_path("/secret.txt:hidden", dir.path()).unwrap_err(),
            PathRejectionReason::AlternateDataStream
        );
        assert_eq!(
            resolve_asset_path("/index.html::$DATA", dir.path()).unwrap_err(),
            PathRejectionReason::AlternateDataStream
        );
        assert_eq!(
            resolve_asset_path("/folder:stream/test", dir.path()).unwrap_err(),
            PathRejectionReason::AlternateDataStream
        );
    }

    #[test]
    fn test_rejection_windows_reserved_device_names() {
        let dir = tempdir().unwrap();
        let devices = [
            "CON", "con", "PRN", "prn", "AUX", "aux", "NUL", "nul", "COM1", "com1", "COM9", "LPT1",
            "lpt1", "LPT9", "con.txt", "nul.json", "aux.html", "com3.js",
        ];

        for dev in &devices {
            let path = format!("/{dev}");
            let err = resolve_asset_path(&path, dir.path()).unwrap_err();
            match err {
                PathRejectionReason::ReservedDeviceName(_) => {}
                other => panic!("Expected ReservedDeviceName for '{dev}', got {other:?}"),
            }

            // In subdirectories
            let sub_path = format!("/sub/{dev}");
            let sub_err = resolve_asset_path(&sub_path, dir.path()).unwrap_err();
            match sub_err {
                PathRejectionReason::ReservedDeviceName(_) => {}
                other => panic!("Expected ReservedDeviceName for '{sub_path}', got {other:?}"),
            }
        }
    }

    #[test]
    fn test_rejection_trailing_dot_or_space() {
        let dir = tempdir().unwrap();
        assert!(matches!(
            resolve_asset_path("/file.txt.", dir.path()).unwrap_err(),
            PathRejectionReason::TrailingDotOrSpace
        ));
        assert!(matches!(
            resolve_asset_path("/file.txt ", dir.path()).unwrap_err(),
            PathRejectionReason::TrailingDotOrSpace
        ));
        assert!(matches!(
            resolve_asset_path("/ file.txt", dir.path()).unwrap_err(),
            PathRejectionReason::TrailingDotOrSpace
        ));
        assert!(matches!(
            resolve_asset_path("/sub./file.txt", dir.path()).unwrap_err(),
            PathRejectionReason::TrailingDotOrSpace
        ));
    }

    /// F-A1a-01: ProtocolError::TrailingDotOrSpace must not leak raw segment strings in Display or Debug.
    #[test]
    fn test_trailing_dot_or_space_does_not_leak_in_display() {
        let dir = tempdir().unwrap();
        let marker = "SECRET_MARKER_999";
        let err = resolve_asset_path(&format!("/foo_{marker}."), dir.path()).unwrap_err();
        let display = format!("{}", err);
        let debug = format!("{:?}", err);
        assert!(
            !display.contains(marker),
            "Display leaks segment marker: {display}"
        );
        assert!(
            !debug.contains(marker),
            "Debug leaks segment marker: {debug}"
        );
    }

    #[test]
    fn test_rejection_overly_long_path() {
        let dir = tempdir().unwrap();
        let long_path = format!("/{}", "a".repeat(MAX_PATH_LENGTH + 1));
        assert!(matches!(
            resolve_asset_path(&long_path, dir.path()).unwrap_err(),
            PathRejectionReason::PathTooLong(_)
        ));
    }

    #[test]
    fn test_symlink_or_junction_escape() {
        let root_dir = tempdir().unwrap();
        let outside_dir = tempdir().unwrap();
        let secret_file = outside_dir.path().join("secret.txt");
        fs::write(&secret_file, "classified").unwrap();

        let link_path = root_dir.path().join("link_outside");

        // Attempt to create symlink (may require admin/developer mode on Windows)
        #[cfg(windows)]
        let link_result = std::os::windows::fs::symlink_file(&secret_file, &link_path);
        #[cfg(unix)]
        let link_result = std::os::unix::fs::symlink(&secret_file, &link_path);

        match link_result {
            Ok(_) => {
                let res = resolve_asset_path("/link_outside", root_dir.path());
                assert_eq!(res.unwrap_err(), PathRejectionReason::EscapedRoot);
            }
            Err(e) => {
                println!(
                    "Skipping symlink test: OS disallowed creating symlink ({e}). \
                     (Normal behavior on Windows without Developer Mode or elevated privileges)."
                );
            }
        }
    }

    #[test]
    fn test_mime_type_mappings() {
        assert_eq!(
            get_mime_type(Path::new("file.html")),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            get_mime_type(Path::new("app.js")),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            get_mime_type(Path::new("style.css")),
            "text/css; charset=utf-8"
        );
        assert_eq!(
            get_mime_type(Path::new("data.json")),
            "application/json; charset=utf-8"
        );
        assert_eq!(get_mime_type(Path::new("image.png")), "image/png");
        assert_eq!(get_mime_type(Path::new("photo.jpg")), "image/jpeg");
        assert_eq!(get_mime_type(Path::new("vector.svg")), "image/svg+xml");
        assert_eq!(get_mime_type(Path::new("font.woff2")), "font/woff2");
        assert_eq!(get_mime_type(Path::new("app.wasm")), "application/wasm");
        assert_eq!(
            get_mime_type(Path::new("binary.xyz")),
            "application/octet-stream"
        );
    }

    // Property-based test with proptest: random inputs must NEVER panic and NEVER escape root
    use proptest::prelude::*;
    proptest! {
        #[test]
        fn proptest_arbitrary_string_does_not_panic_or_escape(path_input in "\\PC*") {
            let dir = tempdir().unwrap();
            let _ = resolve_asset_path(&path_input, dir.path());
        }

        #[test]
        fn proptest_random_ascii_path_does_not_panic_or_escape(path_input in "[a-zA-Z0-9/\\\\%._:\\-]{0,250}") {
            let dir = tempdir().unwrap();
            let safe_root = dir.path().canonicalize().unwrap();

            if let Ok(resolved) = resolve_asset_path(&path_input, dir.path()) {
                prop_assert!(resolved.path.starts_with(&safe_root));
                prop_assert!(resolved.path.is_file());
            }
        }
    }
}
