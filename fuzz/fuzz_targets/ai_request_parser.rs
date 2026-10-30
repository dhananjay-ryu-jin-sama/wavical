#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(utf8_str) = std::str::from_utf8(data) {
        // 1. Fuzz ChatRequest deserialization and validation
        if let Ok(chat_req) = serde_json::from_str::<wavical_ipc::ChatRequest>(utf8_str) {
            let _ = wavical_ipc::ai::validate_chat_request(&chat_req);
        }

        // 2. Fuzz IndexRequest deserialization and validation
        if let Ok(index_req) = serde_json::from_str::<wavical_ipc::IndexRequest>(utf8_str) {
            let _ = wavical_ipc::ai::validate_index_request(&index_req);
        }

        // 3. Fuzz SearchRequest deserialization and validation
        if let Ok(search_req) = serde_json::from_str::<wavical_ipc::SearchRequest>(utf8_str) {
            let _ = wavical_ipc::ai::validate_search_request(&search_req);
        }

        // 4. Fuzz standalone collection name validator
        let _ = wavical_ipc::ai::validate_collection_name(utf8_str);

        // 5. Fuzz standalone document ID validator
        let _ = wavical_ipc::ai::validate_document_id(utf8_str);

        // 6. Fuzz standalone catalog model ID validator
        let _ = wavical_ipc::ai::validate_catalog_model_id(utf8_str);
    }
});

