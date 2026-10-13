#![no_main]

use libfuzzer_sys::fuzz_target;
use wavical_ipc::parse_request;

fuzz_target!(|data: &[u8]| {
    if let Ok(utf8_str) = std::str::from_utf8(data) {
        // Assert that parsing arbitrary UTF-8 never panics
        let _ = parse_request(utf8_str);
    }
});
