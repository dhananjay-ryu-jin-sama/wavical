// tests/audit/tests/part1_protocol.rs
//! Independent black-box security audit test suite for `wavical-protocol`.
//! Expectations are derived strictly from WP3 and the Phase 0 build plan:
//! - docs/phase 0/WAVICAL_PHASE_0_BUILD_PLAN.md
//! - Never derived from what the code currently returns.

use proptest::prelude::*;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use tempfile::{TempDir, tempdir};
use wavical_protocol::{
    HEADER_NOSNIFF, PathRejectionReason, get_mime_type, is_windows_reserved_device,
    resolve_asset_path, sanitize_path_segments,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedOutcome {
    Rejected,
    ResolvedInsideRoot,
}

pub struct TestCase {
    pub name: &'static str,
    pub payload: &'static str,
    pub expected: ExpectedOutcome,
    pub group: &'static str,
}

fn setup_test_root() -> (TempDir, PathBuf) {
    let dir = tempdir().expect("failed to create temp dir");
    let root = dir.path().to_path_buf();

    File::create(root.join("index.html"))
        .unwrap()
        .write_all(b"<html>Index</html>")
        .unwrap();
    File::create(root.join("app.js"))
        .unwrap()
        .write_all(b"console.log('app');")
        .unwrap();
    File::create(root.join("style.css"))
        .unwrap()
        .write_all(b"body { color: red; }")
        .unwrap();

    let assets = root.join("assets");
    fs::create_dir_all(&assets).unwrap();
    File::create(assets.join("sub.html"))
        .unwrap()
        .write_all(b"Sub")
        .unwrap();

    let sub = root.join("sub");
    fs::create_dir_all(&sub).unwrap();
    File::create(sub.join("index.html"))
        .unwrap()
        .write_all(b"Sub Index")
        .unwrap();

    (dir, root)
}

pub fn get_table_cases() -> Vec<TestCase> {
    vec![
        // ==========================================
        // Group 1: Dot segments (Directory Traversal)
        // ==========================================
        TestCase {
            name: "dot_parent_only",
            payload: "..",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_slash_x",
            payload: "../x",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_backslash_x",
            payload: r"..\x",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_nested_traversal",
            payload: "a/../../x",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_trailing_parent",
            payload: "a/..",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_current_parent_slash",
            payload: "./../x",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_quad_slash",
            payload: "....//x",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_semicolon_slash",
            payload: "..;/x",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_single_encoded",
            payload: ".%2e/x",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_percent_2e_2e",
            payload: "%2e%2e/x",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_percent_2E_2E_upper",
            payload: "%2E%2E%2Fx",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_percent_2e_backslash",
            payload: r"%2e%2e\x",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_percent_encoded_backslash",
            payload: "%2e%2e%5cx",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_double_encoded",
            payload: "%252e%252e%252f",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_triple_encoded",
            payload: "%25252e%25252e%25252f",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_overlong_utf8_dots",
            payload: "%c0%ae%c0%ae/",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_overlong_utf8_slash",
            payload: "%c0%af",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_fullwidth_dots",
            payload: "%ef%bc%8e%ef%bc%8e/",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_mixed_slashes",
            payload: r"..\/",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        TestCase {
            name: "dot_triple_dot",
            payload: ".../x",
            expected: ExpectedOutcome::Rejected,
            group: "dot_segments",
        },
        // ==========================================
        // Group 2: Absolute and drive paths
        // ==========================================
        TestCase {
            name: "abs_unix_passwd",
            payload: "/etc/passwd",
            expected: ExpectedOutcome::Rejected,
            group: "absolute_drive",
        },
        TestCase {
            name: "abs_win_drive_backslash",
            payload: r"C:\Windows\win.ini",
            expected: ExpectedOutcome::Rejected,
            group: "absolute_drive",
        },
        TestCase {
            name: "abs_win_drive_slash",
            payload: "C:/Windows/win.ini",
            expected: ExpectedOutcome::Rejected,
            group: "absolute_drive",
        },
        TestCase {
            name: "abs_drive_relative",
            payload: "c:x",
            expected: ExpectedOutcome::Rejected,
            group: "absolute_drive",
        },
        TestCase {
            name: "abs_drive_d_colon",
            payload: "D:/secret",
            expected: ExpectedOutcome::Rejected,
            group: "absolute_drive",
        },
        TestCase {
            name: "abs_unc_backslash",
            payload: r"\\server\share\x",
            expected: ExpectedOutcome::Rejected,
            group: "absolute_drive",
        },
        TestCase {
            name: "abs_unc_slash",
            payload: "//server/share/x",
            expected: ExpectedOutcome::Rejected,
            group: "absolute_drive",
        },
        TestCase {
            name: "abs_unc_win32_device",
            payload: r"\\?\C:\x",
            expected: ExpectedOutcome::Rejected,
            group: "absolute_drive",
        },
        TestCase {
            name: "abs_unc_device_pipe",
            payload: r"\\.\pipe\x",
            expected: ExpectedOutcome::Rejected,
            group: "absolute_drive",
        },
        TestCase {
            name: "abs_unc_device_drive",
            payload: r"\\.\C:\x",
            expected: ExpectedOutcome::Rejected,
            group: "absolute_drive",
        },
        TestCase {
            name: "abs_nt_namespace",
            payload: r"\??\C:\x",
            expected: ExpectedOutcome::Rejected,
            group: "absolute_drive",
        },
        TestCase {
            name: "abs_mixed_unc_slash_backslash",
            payload: r"/\\server\share",
            expected: ExpectedOutcome::Rejected,
            group: "absolute_drive",
        },
        TestCase {
            name: "abs_mixed_unc_backslash_slash",
            payload: r"\//server/share",
            expected: ExpectedOutcome::Rejected,
            group: "absolute_drive",
        },
        // ==========================================
        // Group 3: NTFS Alternate Data Streams (ADS)
        // ==========================================
        TestCase {
            name: "stream_simple",
            payload: "file.txt:stream",
            expected: ExpectedOutcome::Rejected,
            group: "streams",
        },
        TestCase {
            name: "stream_default_data",
            payload: "file.txt::$DATA",
            expected: ExpectedOutcome::Rejected,
            group: "streams",
        },
        TestCase {
            name: "stream_named_data",
            payload: "file.txt:stream:$DATA",
            expected: ExpectedOutcome::Rejected,
            group: "streams",
        },
        TestCase {
            name: "stream_index_allocation",
            payload: "index.html::$INDEX_ALLOCATION",
            expected: ExpectedOutcome::Rejected,
            group: "streams",
        },
        TestCase {
            name: "stream_colon_in_subfolder",
            payload: "assets/index.html:stream",
            expected: ExpectedOutcome::Rejected,
            group: "streams",
        },
        // ==========================================
        // Group 4: Reserved device names
        // ==========================================
        TestCase {
            name: "res_con",
            payload: "CON",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_prn",
            payload: "PRN",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_aux",
            payload: "AUX",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_nul",
            payload: "NUL",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_com1",
            payload: "COM1",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_com2",
            payload: "COM2",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_com3",
            payload: "COM3",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_com4",
            payload: "COM4",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_com5",
            payload: "COM5",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_com6",
            payload: "COM6",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_com7",
            payload: "COM7",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_com8",
            payload: "COM8",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_com9",
            payload: "COM9",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_lpt1",
            payload: "LPT1",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_lpt2",
            payload: "LPT2",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_lpt3",
            payload: "LPT3",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_lpt4",
            payload: "LPT4",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_lpt5",
            payload: "LPT5",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_lpt6",
            payload: "LPT6",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_lpt7",
            payload: "LPT7",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_lpt8",
            payload: "LPT8",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_lpt9",
            payload: "LPT9",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_con_ext",
            payload: "con.txt",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_nul_dot",
            payload: "NUL.",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_sub_con",
            payload: "a/CON",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_com_superscript1",
            payload: "COM\u{00B9}",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_lpt_superscript2",
            payload: "LPT\u{00B2}",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_conin",
            payload: "CONIN$",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_conout",
            payload: "CONOUT$",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        TestCase {
            name: "res_aux_json",
            payload: "sub/AUX.json",
            expected: ExpectedOutcome::Rejected,
            group: "reserved_names",
        },
        // ==========================================
        // Group 5: Trailing dots and spaces
        // ==========================================
        TestCase {
            name: "trail_dot",
            payload: "x.",
            expected: ExpectedOutcome::Rejected,
            group: "trailing_dots_spaces",
        },
        TestCase {
            name: "trail_space",
            payload: "x ",
            expected: ExpectedOutcome::Rejected,
            group: "trailing_dots_spaces",
        },
        TestCase {
            name: "trail_dot_space",
            payload: "x. .",
            expected: ExpectedOutcome::Rejected,
            group: "trailing_dots_spaces",
        },
        TestCase {
            name: "trail_dir_dot_slash",
            payload: "dir./x",
            expected: ExpectedOutcome::Rejected,
            group: "trailing_dots_spaces",
        },
        TestCase {
            name: "trail_dir_space_slash",
            payload: "sub /file.txt",
            expected: ExpectedOutcome::Rejected,
            group: "trailing_dots_spaces",
        },
        TestCase {
            name: "trail_index_dot",
            payload: "index.html.",
            expected: ExpectedOutcome::Rejected,
            group: "trailing_dots_spaces",
        },
        TestCase {
            name: "trail_index_space",
            payload: "index.html ",
            expected: ExpectedOutcome::Rejected,
            group: "trailing_dots_spaces",
        },
        // ==========================================
        // Group 6: Control and encoding problems
        // ==========================================
        TestCase {
            name: "enc_encoded_null",
            payload: "%00",
            expected: ExpectedOutcome::Rejected,
            group: "control_encoding",
        },
        TestCase {
            name: "enc_filename_null",
            payload: "x%00.png",
            expected: ExpectedOutcome::Rejected,
            group: "control_encoding",
        },
        TestCase {
            name: "enc_raw_null",
            payload: "\0",
            expected: ExpectedOutcome::Rejected,
            group: "control_encoding",
        },
        TestCase {
            name: "enc_newline",
            payload: "%0a",
            expected: ExpectedOutcome::Rejected,
            group: "control_encoding",
        },
        TestCase {
            name: "enc_crlf",
            payload: "%0d%0a",
            expected: ExpectedOutcome::Rejected,
            group: "control_encoding",
        },
        TestCase {
            name: "enc_invalid_utf8_ff",
            payload: "%ff",
            expected: ExpectedOutcome::Rejected,
            group: "control_encoding",
        },
        TestCase {
            name: "enc_invalid_utf8_c3_28",
            payload: "%c3%28",
            expected: ExpectedOutcome::Rejected,
            group: "control_encoding",
        },
        TestCase {
            name: "enc_lone_percent",
            payload: "lone%",
            expected: ExpectedOutcome::Rejected,
            group: "control_encoding",
        },
        TestCase {
            name: "enc_invalid_hex",
            payload: "%zz",
            expected: ExpectedOutcome::Rejected,
            group: "control_encoding",
        },
        TestCase {
            name: "enc_incomplete_percent",
            payload: "%2",
            expected: ExpectedOutcome::Rejected,
            group: "control_encoding",
        },
        TestCase {
            name: "enc_rtl_override",
            payload: "%e2%80%ae",
            expected: ExpectedOutcome::Rejected,
            group: "control_encoding",
        },
        TestCase {
            name: "enc_zero_width_space",
            payload: "%e2%80%8b",
            expected: ExpectedOutcome::Rejected,
            group: "control_encoding",
        },
        // ==========================================
        // Group 7: Size and shape
        // ==========================================
        TestCase {
            name: "shape_empty",
            payload: "",
            expected: ExpectedOutcome::ResolvedInsideRoot,
            group: "size_shape",
        },
        TestCase {
            name: "shape_root_slash",
            payload: "/",
            expected: ExpectedOutcome::ResolvedInsideRoot,
            group: "size_shape",
        },
        TestCase {
            name: "shape_double_slash",
            payload: "//",
            expected: ExpectedOutcome::Rejected,
            group: "size_shape",
        },
        TestCase {
            name: "shape_triple_slash",
            payload: "///",
            expected: ExpectedOutcome::Rejected,
            group: "size_shape",
        },
        // ==========================================
        // Group 8: URL parts (query and fragment)
        // ==========================================
        TestCase {
            name: "url_query_traversal",
            payload: "index.html?x=../../a",
            expected: ExpectedOutcome::Rejected,
            group: "url_parts",
        },
        TestCase {
            name: "url_fragment_traversal",
            payload: "index.html#../x",
            expected: ExpectedOutcome::Rejected,
            group: "url_parts",
        },
        // ==========================================
        // Group 9: 8.3 short names
        // ==========================================
        TestCase {
            name: "short_83_progra",
            payload: "PROGRA~1",
            expected: ExpectedOutcome::Rejected,
            group: "short_names",
        },
        TestCase {
            name: "short_83_progra_sub",
            payload: "PROGRA~1/x",
            expected: ExpectedOutcome::Rejected,
            group: "short_names",
        },
        // ==========================================
        // Group 10: Case sensitivity (Windows NTFS)
        // ==========================================
        TestCase {
            name: "case_upper_index",
            payload: "INDEX.HTML",
            expected: ExpectedOutcome::ResolvedInsideRoot,
            group: "case",
        },
        TestCase {
            name: "case_mixed_index",
            payload: "InDeX.hTmL",
            expected: ExpectedOutcome::ResolvedInsideRoot,
            group: "case",
        },
        TestCase {
            name: "case_upper_style",
            payload: "STYLE.CSS",
            expected: ExpectedOutcome::ResolvedInsideRoot,
            group: "case",
        },
        // ==========================================
        // Group 11: Valid standard assets
        // ==========================================
        TestCase {
            name: "valid_index",
            payload: "index.html",
            expected: ExpectedOutcome::ResolvedInsideRoot,
            group: "valid",
        },
        TestCase {
            name: "valid_app_js",
            payload: "app.js",
            expected: ExpectedOutcome::ResolvedInsideRoot,
            group: "valid",
        },
        TestCase {
            name: "valid_style_css",
            payload: "style.css",
            expected: ExpectedOutcome::ResolvedInsideRoot,
            group: "valid",
        },
        TestCase {
            name: "valid_sub_html",
            payload: "assets/sub.html",
            expected: ExpectedOutcome::ResolvedInsideRoot,
            group: "valid",
        },
        TestCase {
            name: "valid_sub_dir_index",
            payload: "sub/",
            expected: ExpectedOutcome::ResolvedInsideRoot,
            group: "valid",
        },
    ]
}

#[test]
fn test_part1_table_driven_suite() {
    let (_temp_guard, root) = setup_test_root();
    let cases = get_table_cases();
    assert!(
        cases.len() >= 80,
        "Table must contain at least 80 cases (had {})",
        cases.len()
    );

    let mut failed_cases = Vec::new();

    for case in &cases {
        let res = resolve_asset_path(case.payload, &root);
        match case.expected {
            ExpectedOutcome::Rejected => {
                if let Ok(resolved) = res {
                    failed_cases.push((
                        case.name,
                        case.payload,
                        case.group,
                        format!("Expected Rejected, but resolved to {:?}", resolved.path),
                    ));
                }
            }
            ExpectedOutcome::ResolvedInsideRoot => match res {
                Ok(resolved) => {
                    let canon_root = root.canonicalize().unwrap();
                    if !resolved.path.starts_with(&canon_root) {
                        failed_cases.push((
                            case.name,
                            case.payload,
                            case.group,
                            format!(
                                "Resolved path {:?} is outside root {:?}",
                                resolved.path, canon_root
                            ),
                        ));
                    }
                }
                Err(err) => {
                    failed_cases.push((
                        case.name,
                        case.payload,
                        case.group,
                        format!("Expected ResolvedInsideRoot, but failed with {:?}", err),
                    ));
                }
            },
        }
    }

    if !failed_cases.is_empty() {
        println!("=== TABLE-DRIVEN FAILURES (Findings) ===");
        for (name, payload, group, msg) in &failed_cases {
            println!("[FINDING] {name} (group: {group}, payload: {payload:?}): {msg}");
        }
        panic!("{} table test cases failed", failed_cases.len());
    }
}

#[test]
fn test_part1_size_and_shape_long_paths() {
    let (_temp_guard, root) = setup_test_root();

    // 1. Path longer than 260 characters (Windows legacy MAX_PATH)
    // If not existing, must return NotFound or PathTooLong, never crash or leak
    let long_261 = format!("sub/{}.html", "a".repeat(261));
    assert!(resolve_asset_path(&long_261, &root).is_err());

    // 2. Path longer than 1,000 characters
    let long_1000 = format!("sub/{}.html", "a".repeat(1000));
    assert!(resolve_asset_path(&long_1000, &root).is_err());

    // 3. Path longer than 100,000 characters (exceeds MAX_PATH_LENGTH of 2048)
    let huge_path = "a/".repeat(50_000);
    assert_eq!(huge_path.len(), 100_000);
    let huge_res = resolve_asset_path(&huge_path, &root);
    assert!(huge_res.is_err());

    // 4. 1,000 nested segments: a/a/a/.../index.html (2,010 chars <= 2048)
    let nested_1000 = format!("{}index.html", "a/".repeat(1000));
    let res_nested = resolve_asset_path(&nested_1000, &root);
    assert!(res_nested.is_err());
}

#[test]
fn test_part1_mime_detection_and_nosniff() {
    // Assert nosniff header
    assert_eq!(HEADER_NOSNIFF, ("X-Content-Type-Options", "nosniff"));

    // Expected MIME map
    let mime_cases = [
        ("index.html", "text/html; charset=utf-8"),
        ("app.js", "text/javascript; charset=utf-8"),
        ("module.mjs", "text/javascript; charset=utf-8"),
        ("style.css", "text/css; charset=utf-8"),
        ("data.json", "application/json; charset=utf-8"),
        ("logo.png", "image/png"),
        ("icon.svg", "image/svg+xml"),
        ("module.wasm", "application/wasm"),
        ("favicon.ico", "image/x-icon"),
        ("font.woff2", "font/woff2"),
        // Unknown extension must default to a safe binary type, NEVER text/html
        ("binary.xyz", "application/octet-stream"),
        ("unknown_file", "application/octet-stream"),
        ("script.php", "application/octet-stream"),
        ("document.unknown_ext", "application/octet-stream"),
    ];

    for (filename, expected_mime) in mime_cases {
        let detected = get_mime_type(Path::new(filename));
        assert_eq!(
            detected, expected_mime,
            "MIME type mismatch for {filename}: expected {expected_mime}, got {detected}"
        );
        assert_ne!(
            detected, "text/html",
            "MIME type for unknown/binary file {filename} must never be text/html"
        );
    }
}

#[test]
fn test_part1_filesystem_escapes() {
    let (_temp_guard, root) = setup_test_root();

    // Create an outside secret file
    let outside_dir = tempdir().expect("failed to create outside dir");
    let outside_secret = outside_dir.path().join("secret.txt");
    File::create(&outside_secret)
        .unwrap()
        .write_all(b"CONFIDENTIAL_OUTSIDE_DATA")
        .unwrap();

    // 1. Symlink escape
    let symlink_path = root.join("symlink_outside.txt");
    #[cfg(windows)]
    let symlink_created =
        std::os::windows::fs::symlink_file(&outside_secret, &symlink_path).is_ok();
    #[cfg(unix)]
    let symlink_created = std::os::unix::fs::symlink(&outside_secret, &symlink_path).is_ok();

    if symlink_created {
        let res = resolve_asset_path("symlink_outside.txt", &root);
        assert!(
            res.is_err(),
            "Symlink pointing outside root must be rejected, but got {:?}",
            res
        );
    } else {
        println!("INFO: Symlink creation skipped (requires Windows Developer Mode or privileges)");
    }

    // 2. Directory Junction escape (Windows mklink /J)
    #[cfg(windows)]
    {
        let junction_path = root.join("junction_outside");
        let output = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction_path)
            .arg(outside_dir.path())
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let res = resolve_asset_path("junction_outside/secret.txt", &root);
                assert!(
                    res.is_err(),
                    "Junction pointing outside root must be rejected, but resolved to {:?}",
                    res
                );
            }
            Ok(out) => {
                println!(
                    "INFO: Directory junction creation failed with exit code {:?}: {}",
                    out.status.code(),
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            Err(e) => {
                println!("INFO: Directory junction command failed: {e}");
            }
        }
    }

    // 3. Hard link to outside file (Report behavior)
    let hardlink_path = root.join("hardlink_outside.txt");
    let hardlink_created = fs::hard_link(&outside_secret, &hardlink_path).is_ok();
    if hardlink_created {
        let res = resolve_asset_path("hardlink_outside.txt", &root);
        // Canonicalize returns the path inside the root for hardlinks because they share the same MFT entry
        println!("BEHAVIOR REPORT: Hard link resolution result: {:?}", res);
    } else {
        println!("INFO: Hard link across volumes/tempfiles not supported or failed");
    }

    // 4. Directory named like a file (app_dir.js without index.html)
    let fake_dir = root.join("app_dir.js");
    fs::create_dir_all(&fake_dir).unwrap();
    let res_dir = resolve_asset_path("app_dir.js", &root);
    assert!(
        res_dir.is_err(),
        "Directory without index.html must be rejected, got {:?}",
        res_dir
    );
}

/// Error values must NEVER leak the absolute root path or raw requested path in Display or Debug.
/// FINDING-P1: PathRejectionReason::TrailingDotOrSpace and ReservedDeviceName leak raw segment strings in Display output.
#[test]
fn test_part1_error_information_leakage() {
    let secret_root_marker = "SECRET_ROOT_MARKER_98765";
    let dir = tempfile::Builder::new()
        .prefix(secret_root_marker)
        .tempdir()
        .unwrap();
    let root = dir.path();

    File::create(root.join("index.html"))
        .unwrap()
        .write_all(b"Index")
        .unwrap();

    let secret_raw_marker = "SECRET_RAW_MARKER_12345";
    let test_payloads = [
        format!("file_{secret_raw_marker} .txt"),
        format!("x_{secret_raw_marker}."),
        format!("CON_{secret_raw_marker}"),
        format!("nonexistent_{secret_raw_marker}.html"),
    ];

    for payload in &test_payloads {
        if let Err(err) = resolve_asset_path(payload, root) {
            let display_str = format!("{}", err);
            let debug_str = format!("{:?}", err);

            assert!(
                !display_str.contains(secret_root_marker),
                "Error Display leaks secret root marker: {display_str}"
            );
            assert!(
                !debug_str.contains(secret_root_marker),
                "Error Debug leaks secret root marker: {debug_str}"
            );
            assert!(
                !display_str.contains(secret_raw_marker),
                "Error Display leaks raw requested path: {display_str}"
            );
            assert!(
                !debug_str.contains(secret_raw_marker),
                "Error Debug leaks raw requested path: {debug_str}"
            );
        }
    }
}

/// Windows special console devices `CONIN$` and `CONOUT$` are reserved names in the Windows console subsystem.
/// FINDING-P2 / F-A1a-02: `is_windows_reserved_device` must match CONIN$ and CONOUT$.
#[test]
fn test_part1_reserved_device_conin_conout() {
    assert!(
        is_windows_reserved_device("CONIN$"),
        "CONIN$ must be detected as reserved device"
    );
    assert!(
        is_windows_reserved_device("CONOUT$"),
        "CONOUT$ must be detected as reserved device"
    );
}

/// F-A5-02: Raw unencoded null byte pre-check in resolve_asset_path must reject with PathRejectionReason::NullByte
#[test]
fn test_resolve_asset_path_unencoded_null_byte_rejected() {
    let (_temp_guard, root) = setup_test_root();
    let res = resolve_asset_path("index.html\0.png", &root);
    assert_eq!(
        res.unwrap_err(),
        PathRejectionReason::NullByte,
        "F-A5-02: Passing unencoded raw null byte 'index.html\\0.png' must return PathRejectionReason::NullByte"
    );
}

/// F-A5-03: Raw UNC share prefix check in sanitize_path_segments must trigger early rejection
/// at Step 3 (UncPath) before Step 4 percent-decoding runs.
#[test]
fn test_raw_unc_share_prefix_check_step3() {
    assert_eq!(
        sanitize_path_segments("//server/share"),
        Err(PathRejectionReason::UncPath),
        "F-A5-03: Raw '//server/share' must be rejected with UncPath"
    );
    assert_eq!(
        sanitize_path_segments(r"\\server\share"),
        Err(PathRejectionReason::UncPath),
        "F-A5-03: Raw '\\\\server\\share' must be rejected with UncPath"
    );
    // Early Step 3 check must reject UNC before Step 4 percent-decoding:
    // If Step 3 were bypassed, Step 4 percent-decoding would fail on '%ZZ' with InvalidEncoding.
    assert_eq!(
        sanitize_path_segments("//server/share%ZZ"),
        Err(PathRejectionReason::UncPath),
        "F-A5-03: Early Step 3 check must reject UNC before Step 4 percent-decoding"
    );
}

/// F-A1a-04: Secondary MIME Type Match Arms (gif, webp, ico, woff, ttf) must be tested in wavical-protocol.
/// Verifies:
/// 1. crates/wavical-protocol/src/lib.rs::test_mime_type_mappings contains explicit assertions for secondary extensions.
/// 2. get_mime_type correctly maps all secondary MIME types.
#[test]
fn test_secondary_mime_type_match_arms_tested() {
    let protocol_lib = include_str!("../../../crates/wavical-protocol/src/lib.rs");
    let required_extensions = [
        "gif", "webp", "ico", "woff", "ttf", "otf", "htm", "mjs", "jpeg", "txt", "pdf",
    ];

    // 1. Verify test_mime_type_mappings in crates/wavical-protocol/src/lib.rs covers all secondary extensions
    let test_section = protocol_lib
        .split("fn test_mime_type_mappings()")
        .nth(1)
        .expect("test_mime_type_mappings must exist");
    let test_body = test_section
        .split('}')
        .next()
        .expect("test body must end with }");

    for ext in required_extensions {
        assert!(
            test_body.contains(ext),
            "F-A1a-04: test_mime_type_mappings in crates/wavical-protocol/src/lib.rs must assert extension '{ext}'"
        );
    }

    // 2. Functional verification of all secondary MIME types
    let cases = [
        ("image.gif", "image/gif"),
        ("image.webp", "image/webp"),
        ("favicon.ico", "image/x-icon"),
        ("font.woff", "font/woff"),
        ("font.ttf", "font/ttf"),
        ("font.otf", "font/otf"),
        ("index.htm", "text/html; charset=utf-8"),
        ("module.mjs", "text/javascript; charset=utf-8"),
        ("photo.jpeg", "image/jpeg"),
        ("readme.txt", "text/plain; charset=utf-8"),
        ("doc.pdf", "application/pdf"),
    ];

    for (file, expected_mime) in cases {
        let detected = get_mime_type(Path::new(file));
        assert_eq!(
            detected, expected_mime,
            "F-A1a-04: get_mime_type({file}) must return {expected_mime}"
        );
    }
}

// =========================================================================
// Proptests: at least 10,000 cases per property
// =========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    /// Property 1: Any arbitrary string never panics the path resolver
    #[test]
    fn prop_path_resolver_never_panics(s in "\\PC*") {
        let dir = tempdir().unwrap();
        File::create(dir.path().join("index.html")).unwrap();
        let _ = resolve_asset_path(&s, dir.path());
    }

    /// Property 2: Any Ok result is canonically inside the asset root
    #[test]
    fn prop_path_resolver_ok_strictly_inside_root(s in "[a-zA-Z0-9_./\\%-]{0,60}") {
        let dir = tempdir().unwrap();
        let root = dir.path();
        File::create(root.join("index.html")).unwrap();
        File::create(root.join("app.js")).unwrap();
        let canon_root = root.canonicalize().unwrap();

        if let Ok(resolved) = resolve_asset_path(&s, root) {
            prop_assert!(
                resolved.path.starts_with(&canon_root),
                "Resolved path {:?} escaped root {:?}",
                resolved.path,
                canon_root
            );
        }
    }

    /// Property 3: No resolved result contains '..' components
    #[test]
    fn prop_path_resolver_no_parent_dir_component(s in "[a-zA-Z0-9_./\\%-]{0,60}") {
        let dir = tempdir().unwrap();
        let root = dir.path();
        File::create(root.join("index.html")).unwrap();
        let canon_root = root.canonicalize().unwrap();

        if let Ok(resolved) = resolve_asset_path(&s, root) {
            let rel = resolved.path.strip_prefix(&canon_root).unwrap_or(&resolved.path);
            for comp in rel.components() {
                prop_assert_ne!(
                    comp,
                    Component::ParentDir,
                    "Resolved path contains '..' parent dir component: {:?}",
                    resolved.path
                );
            }
        }
    }
}
