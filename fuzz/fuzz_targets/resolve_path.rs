#![no_main]

use libfuzzer_sys::fuzz_target;
use std::path::Path;
use wavical_protocol::resolve_asset_path;

fuzz_target!(|data: &[u8]| {
    if let Ok(path_str) = std::str::from_utf8(data) {
        // Asset root fixture directory
        let asset_root = Path::new("examples/hello");
        // Assert that path resolution on arbitrary input never panics
        let _ = resolve_asset_path(path_str, asset_root);
    }
});
