use std::fs;
use std::io::{Cursor, Read, Write};
use std::net::TcpListener;
use std::thread;

use tempfile::TempDir;
use wavical_ai::downloader::DownloadOptions;
use wavical_cli::html_scan::{extract_frontend_inline_scripts, extract_inline_scripts_from_html};
use wavical_cli::vendor::{VendorRunOptions, run_vendor_with_io, url_to_vendor_rel_path};
use wavical_netpolicy::limits::{MAX_VENDOR_FILE_BYTES, MAX_VENDOR_FILE_COUNT};
use wavical_netpolicy::vendor_lock::VendorLock;

fn spawn_fixture_server(routes: Vec<(String, Vec<u8>, &'static str)>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut s = stream;
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let req_path = req
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("/");
            if let Some((_, body, ctype)) = routes.iter().find(|(p, _, _)| p == req_path) {
                let hdr = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(hdr.as_bytes());
                let _ = s.write_all(body);
            } else {
                let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            }
        }
    });
    port
}

#[test]
fn vendor_google_fonts_css_pinned_js_check_and_tamper() {
    let port = spawn_fixture_server(vec![
        (
            "/ajax/libs/lodash.js/4.17.21/lodash.min.js".to_string(),
            b"/* SPDX-License-Identifier: MIT */\nwindow._ = {};".to_vec(),
            "application/javascript",
        ),
        (
            "/css2?family=Roboto:wght@400".to_string(),
            b"@font-face { font-family: 'Roboto'; src: url('/s/roboto/v30/roboto.woff2'); }"
                .to_vec(),
            "text/css",
        ),
        (
            "/s/roboto/v30/roboto.woff2".to_string(),
            b"WOFF2DATA".to_vec(),
            "font/woff2",
        ),
    ]);

    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();

    let pinned_js = format!("http://127.0.0.1:{port}/ajax/libs/lodash.js/4.17.21/lodash.min.js");
    let css_url = format!("http://127.0.0.1:{port}/css2?family=Roboto:wght@400");
    let html = format!(
        r#"<!doctype html><html><head>
<link rel="stylesheet" href="{css_url}">
<script src="{pinned_js}"></script>
</head><body></body></html>"#
    );
    fs::write(src.join("index.html"), &html).unwrap();

    let dl_opts = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };

    // 1. Unpinned URL (query-string CSS without version) is refused by default
    let opts_refuse = VendorRunOptions {
        allow_unpinned: vec![],
        check: false,
        update: false,
        download_options: dl_opts,
    };
    let mut empty_in = Cursor::new(b"");
    let mut out = Vec::new();
    let err = run_vendor_with_io(tmp.path(), &opts_refuse, &mut empty_in, &mut out).unwrap_err();
    assert!(err.to_string().contains("unpinned"));

    // 2. With --allow-unpinned for css_url, vendoring succeeds, follows CSS 1 level, writes lock + backup
    let opts_allow = VendorRunOptions {
        allow_unpinned: vec![css_url.clone()],
        check: false,
        update: false,
        download_options: dl_opts,
    };
    let mut out2 = Vec::new();
    run_vendor_with_io(tmp.path(), &opts_allow, &mut empty_in, &mut out2).unwrap();
    let out_str = String::from_utf8(out2).unwrap();
    assert!(out_str.contains("Vendoring redistributes these files inside your app. You are responsible for their licenses."));
    assert!(out_str.contains("MIT"));

    // Backup exists and HTML was rewritten to local vendor/
    assert!(src.join("index.html.bak").is_file());
    let rewritten_html = fs::read_to_string(src.join("index.html")).unwrap();
    assert!(!rewritten_html.contains(&pinned_js));
    assert!(rewritten_html.contains("vendor/"));

    // vendor.lock.json validates with WP2 parser
    let lock_bytes = fs::read(tmp.path().join("vendor.lock.json")).unwrap();
    let lock = VendorLock::parse_bytes_with_options(&lock_bytes, true).unwrap();
    assert_eq!(lock.entries.len(), 3); // css, woff2, js

    // 3. --check passes before modification
    let opts_check = VendorRunOptions {
        check: true,
        download_options: dl_opts,
        ..Default::default()
    };
    let mut out_chk = Vec::new();
    run_vendor_with_io(tmp.path(), &opts_check, &mut empty_in, &mut out_chk).unwrap();

    // 4. Modify 1 byte of a vendored file -> --check fails
    let first_file = src.join(&lock.entries[0].local_path);
    let mut bytes = fs::read(&first_file).unwrap();
    bytes[0] ^= 0xFF;
    fs::write(&first_file, bytes).unwrap();
    let mut out_chk2 = Vec::new();
    assert!(run_vendor_with_io(tmp.path(), &opts_check, &mut empty_in, &mut out_chk2).is_err());
}

#[test]
fn vendor_rejects_css_traversal_oversize_file_and_over_500_files() {
    let dl_opts = DownloadOptions {
        allow_test_localhost: true,
        skip_disk_check: true,
    };

    // 1. CSS url() with ../../ trying to escape
    let port_trav = spawn_fixture_server(vec![(
        "/v1/1.0.0/app.css".to_string(),
        b"@font-face { src: url('../../../../etc/passwd'); }".to_vec(),
        "text/css",
    )]);
    let tmp1 = TempDir::new().unwrap();
    fs::create_dir_all(tmp1.path().join("src")).unwrap();
    fs::write(
        tmp1.path().join("src/index.html"),
        format!(r#"<link rel="stylesheet" href="http://127.0.0.1:{port_trav}/v1/1.0.0/app.css">"#),
    )
    .unwrap();
    let opts1 = VendorRunOptions {
        download_options: dl_opts,
        ..Default::default()
    };
    let mut r = Cursor::new(b"");
    let mut w = Vec::new();
    let err = run_vendor_with_io(tmp1.path(), &opts1, &mut r, &mut w).unwrap_err();
    assert!(err.to_string().contains("traversal"));

    // 2. 20 MB + 1 file rejected by shared verified downloader
    let big_body = vec![b'a'; (MAX_VENDOR_FILE_BYTES as usize) + 1];
    let port_big = spawn_fixture_server(vec![(
        "/v1/1.0.0/big.js".to_string(),
        big_body,
        "application/javascript",
    )]);
    let tmp2 = TempDir::new().unwrap();
    fs::create_dir_all(tmp2.path().join("src")).unwrap();
    fs::write(
        tmp2.path().join("src/index.html"),
        format!(r#"<script src="http://127.0.0.1:{port_big}/v1/1.0.0/big.js"></script>"#),
    )
    .unwrap();
    assert!(run_vendor_with_io(tmp2.path(), &opts1, &mut r, &mut w).is_err());

    // 3. 501 files rejected
    let mut routes = Vec::new();
    let mut tags = String::new();
    let port_many = {
        for i in 0..=MAX_VENDOR_FILE_COUNT {
            routes.push((
                format!("/v1/1.0.0/f{i}.js"),
                b"1;".to_vec(),
                "application/javascript",
            ));
        }
        spawn_fixture_server(routes)
    };
    for i in 0..=MAX_VENDOR_FILE_COUNT {
        tags.push_str(&format!(
            r#"<script src="http://127.0.0.1:{port_many}/v1/1.0.0/f{i}.js"></script>"#
        ));
    }
    let tmp3 = TempDir::new().unwrap();
    fs::create_dir_all(tmp3.path().join("src")).unwrap();
    fs::write(tmp3.path().join("src/index.html"), tags).unwrap();
    let err_many = run_vendor_with_io(tmp3.path(), &opts1, &mut r, &mut w).unwrap_err();
    assert!(err_many.to_string().contains("500"));

    // 4. Query string normalization produces safe path without '?' or '&'
    let norm = url_to_vendor_rel_path(
        "https://fonts.googleapis.com/css2?family=Inter&display=swap",
        "css",
    )
    .unwrap();
    assert!(norm.starts_with("vendor/fonts.googleapis.com/"));
    assert!(!norm.contains('?'));
    assert!(!norm.contains('&'));
}

#[test]
fn inline_extraction_preserves_order_and_module_semantics_and_reports_handlers() {
    let html = r#"<!doctype html>
<html>
<body>
  <img src="x" onerror="this.remove()" style="opacity:0">
  <script>window.a = 1;</script>
  <script type="module">import { x } from './m.js'; window.b = window.a + x;</script>
  <script>window.c = window.a + 2;</script>
</body>
</html>"#;

    let out = extract_inline_scripts_from_html("index.html", html, ".wavical-extracted/");
    assert_eq!(out.extracted_files.len(), 3);
    assert_eq!(out.extracted_files[0].1, "window.a = 1;");
    assert_eq!(
        out.extracted_files[1].1,
        "import { x } from './m.js'; window.b = window.a + x;"
    );
    assert_eq!(out.extracted_files[2].1, "window.c = window.a + 2;");

    assert!(
        out.rewritten_html
            .contains(r#"<script src=".wavical-extracted/index_html_inline_0.js"></script>"#)
    );
    assert!(out.rewritten_html.contains(
        r#"<script type="module" src=".wavical-extracted/index_html_inline_1.js"></script>"#
    ));
    assert!(
        out.rewritten_html
            .contains(r#"<script src=".wavical-extracted/index_html_inline_2.js"></script>"#)
    );
    // Execution order preserved
    let p0 = out.rewritten_html.find("inline_0.js").unwrap();
    let p1 = out.rewritten_html.find("inline_1.js").unwrap();
    let p2 = out.rewritten_html.find("inline_2.js").unwrap();
    assert!(p0 < p1 && p1 < p2);

    // Inline handler reported, not rewritten
    assert_eq!(out.scan.inline_handlers.len(), 1);
    assert_eq!(out.scan.inline_handlers[0].attribute, "onerror");
    assert!(out.rewritten_html.contains(r#"onerror="this.remove()""#));

    // Disk extraction smoke test
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("index.html"), html).unwrap();
    let extracted_dir = src.join(".wavical-extracted");
    let scans =
        extract_frontend_inline_scripts(&src, &extracted_dir, ".wavical-extracted/", true).unwrap();
    assert_eq!(scans.len(), 1);
    assert!(extracted_dir.join("index_html_inline_0.js").is_file());
    assert!(extracted_dir.join("index_html_inline_1.js").is_file());
    assert!(extracted_dir.join("index_html_inline_2.js").is_file());

    // Hostile review fix: commented-out <script> must NOT be extracted
    let commented = "<!-- <script>dead()</script> --><script>live();</script>";
    let out_cmt = extract_inline_scripts_from_html("cmt.html", commented, ".wavical-extracted/");
    assert_eq!(out_cmt.extracted_files.len(), 1);
    assert_eq!(out_cmt.extracted_files[0].1, "live();");
    assert!(
        out_cmt
            .rewritten_html
            .contains("<!-- <script>dead()</script> -->")
    );

    // Hostile review fix: commented-out <script> must NOT be extracted
    let commented = "<!-- <script>dead()</script> --><script>live();</script>";
    let out_cmt = extract_inline_scripts_from_html("cmt.html", commented, ".wavical-extracted/");
    assert_eq!(out_cmt.extracted_files.len(), 1);
    assert_eq!(out_cmt.extracted_files[0].1, "live();");
    assert!(
        out_cmt
            .rewritten_html
            .contains("<!-- <script>dead()</script> -->")
    );
}
