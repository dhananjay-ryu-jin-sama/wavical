#![no_main]

use libfuzzer_sys::fuzz_target;
use wavical_ipc::{parse_update_manifest, validate_manifest_for_install};

fuzz_target!(|data: &[u8]| {
    if let Ok(manifest) = parse_update_manifest(data) {
        let _ = validate_manifest_for_install(
            &manifest,
            "com.wavical.fuzz",
            "stable",
            "1.0.0",
            0,
            1_700_000_000,
        );
    }
});

