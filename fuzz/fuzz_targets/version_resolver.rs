#![no_main]

use libfuzzer_sys::fuzz_target;
use wavical_protocol::{RuntimeArch, SemVer, resolve_runtime_version};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let allow_patch = (data[0] & 1) != 0;
    if let Ok(s) = std::str::from_utf8(&data[1..]) {
        let _ = SemVer::parse(s);
        let _ = resolve_runtime_version("node", s, allow_patch, RuntimeArch::WindowsX64);
    }
});

