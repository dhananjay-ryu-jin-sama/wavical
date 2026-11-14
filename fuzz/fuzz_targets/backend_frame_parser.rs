#![no_main]

use libfuzzer_sys::fuzz_target;
use wavical_ipc::BackendFrameBuffer;

fuzz_target!(|data: &[u8]| {
    let mut resp_buf = BackendFrameBuffer::new();
    if resp_buf.push_bytes(data).is_ok() {
        while let Ok(Some(_)) = resp_buf.pop_response_frame() {}
    }

    let mut req_buf = BackendFrameBuffer::new();
    if req_buf.push_bytes(data).is_ok() {
        while let Ok(Some(_)) = req_buf.pop_request_frame() {}
    }
});

