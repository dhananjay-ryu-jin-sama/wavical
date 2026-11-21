use std::fs;
use std::io::Cursor;

use tempfile::TempDir;
use wavical_cli::allowlist_suggest::run_allowlist_suggest_with_io;
use wavical_netpolicy::allowlist::Allowlist;

#[test]
fn allowlist_suggest_never_proposes_forbidden_preserves_existing_and_validates() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join(".wavical")).unwrap();

    // Existing allowlist with custom entry order
    let existing = r#"{
  "version": 1,
  "scripts": ["https://existing-first.com", "https://existing-second.com"],
  "styles": []
}"#;
    fs::write(tmp.path().join("wavical.allowlist.json"), existing).unwrap();

    // Recorded blocks including proposable https origins AND forbidden kinds
    let recorded = r#"{
  "version": 1,
  "total_events": 250,
  "entries": [
    { "type": "scripts", "origin": "https://cdn.tailwindcss.com", "count": 2 },
    { "type": "fonts", "origin": "https://fonts.gstatic.com", "count": 4 },
    { "type": "styles", "origin": "inline", "count": 214 },
    { "type": "images", "origin": "data:", "count": 12 },
    { "type": "scripts", "origin": "https://*.wildcard.com", "count": 1 },
    { "type": "connect", "origin": "http://localhost:3000", "count": 5 },
    { "type": "remoteUi", "origin": "https://app.company.com", "count": 1 },
    { "type": "bridge", "origin": "https://app.company.com", "count": 1 },
    { "type": "localExceptions", "origin": "http://127.0.0.1:8080", "count": 1 }
  ]
}"#;
    fs::write(
        tmp.path().join(".wavical").join("recorded-blocks.json"),
        recorded,
    )
    .unwrap();

    // 1. Declining ('n') writes nothing
    let mut input_no = Cursor::new(b"n\n");
    let mut out_no = Vec::new();
    run_allowlist_suggest_with_io(tmp.path(), &mut input_no, &mut out_no).unwrap();
    let out_no_str = String::from_utf8(out_no).unwrap();
    assert!(out_no_str.contains("Needs your decision"));
    assert!(out_no_str.contains("inline styles were blocked 214 time(s)"));
    assert!(out_no_str.contains("data: images were blocked 12 time(s)"));
    assert_eq!(
        fs::read_to_string(tmp.path().join("wavical.allowlist.json")).unwrap(),
        existing
    );

    // 2. Accepting all ('a') writes valid allowlist preserving existing entries & order
    let mut input_yes = Cursor::new(b"a\n");
    let mut out_yes = Vec::new();
    run_allowlist_suggest_with_io(tmp.path(), &mut input_yes, &mut out_yes).unwrap();

    let updated_bytes = fs::read(tmp.path().join("wavical.allowlist.json")).unwrap();
    let parsed = Allowlist::parse_bytes(&updated_bytes).expect("written allowlist must validate");

    // Existing entries preserved in exact order, new entry appended
    assert_eq!(parsed.scripts.len(), 3);
    assert_eq!(
        parsed.scripts[0].as_canonical(),
        "https://existing-first.com"
    );
    assert_eq!(
        parsed.scripts[1].as_canonical(),
        "https://existing-second.com"
    );
    assert_eq!(
        parsed.scripts[2].as_canonical(),
        "https://cdn.tailwindcss.com"
    );
    assert_eq!(parsed.fonts[0].as_canonical(), "https://fonts.gstatic.com");

    // Forbidden kinds were NEVER added
    assert!(!parsed.inline_styles);
    assert!(!parsed.data_images);
    assert!(parsed.local_exceptions.is_empty());
    assert!(parsed.remote_ui.is_none());
}
