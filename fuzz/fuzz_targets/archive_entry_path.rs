#![no_main]

use libfuzzer_sys::fuzz_target;
use std::collections::HashSet;
use wavical_protocol::validate_archive_entry_path;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let flags = data[0];
    let is_symlink = (flags & 1) != 0;
    let is_hardlink = (flags & 2) != 0;
    if let Ok(s) = std::str::from_utf8(&data[1..]) {
        let mut seen = HashSet::new();
        for line in s.lines() {
            let _ = validate_archive_entry_path(line, is_symlink, is_hardlink, &mut seen);
        }
    }
});

