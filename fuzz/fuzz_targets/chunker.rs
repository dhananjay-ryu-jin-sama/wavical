#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }

    let max_chunk = u16::from_le_bytes([data[0], data[1]]) as usize;
    let overlap = u16::from_le_bytes([data[2], data[3]]) as usize;

    if let Ok(utf8_str) = std::str::from_utf8(&data[4..]) {
        let _ = wavical_ai::chunker::chunk_text(utf8_str, max_chunk, overlap);
    }
});

