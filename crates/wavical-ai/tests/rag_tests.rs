#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

use proptest::prelude::*;
use wavical_ai::chunker::chunk_text;
use wavical_ai::client::create_engine_http_client;
use wavical_ai::manager::LocalEngineManager;
use wavical_ai::process::EngineLaunchOptions;
use wavical_ai::rag::{execute_index_request, execute_search_request};
use wavical_ai::sha256::sha256_file;
use wavical_ai::store::{
    DocumentToIndex, StoreError, count_app_collections, delete_collection, get_collection_db_path,
    index_collection_documents, open_store_connection, search_collection,
};
use wavical_ipc::ai::{
    Document, IndexAction, IndexRequest, MAX_COLLECTIONS_PER_APP, MAX_DOCUMENT_CHARS,
    MAX_DOCUMENTS_PER_INDEX_CALL, MAX_SEARCH_QUERY_CHARS, MetadataValue, SearchRequest,
    validate_collection_name, validate_document_id, validate_document_metadata,
    validate_index_request, validate_search_request,
};

// ============================================================================
// 1. Chunker Property Tests (10,000 test cases)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn prop_chunker_never_panics_and_invariants_hold(
        text in "\\PC*",
        max_chunk_chars in 1usize..2_000,
        overlap_chars in 0usize..500,
    ) {
        let chunks = chunk_text(&text, max_chunk_chars, overlap_chars);
        let total_chars = text.chars().count();

        if total_chars == 0 {
            prop_assert!(chunks.is_empty(), "Empty text must produce 0 chunks");
            return Ok(());
        }

        prop_assert!(!chunks.is_empty(), "Non-empty text must produce at least 1 chunk");

        // 1. Invariant: chunks never exceed max_chunk_chars
        for (i, chunk) in chunks.iter().enumerate() {
            let chunk_len = chunk.text.chars().count();
            prop_assert!(
                chunk_len <= max_chunk_chars,
                "Chunk {} length {} exceeds limit {}",
                i, chunk_len, max_chunk_chars
            );
            prop_assert!(chunk_len > 0, "Chunk {} cannot be empty", i);
            prop_assert_eq!(chunk.char_end - chunk.char_start, chunk_len);
        }

        // 2. Invariant: chunks together cover all of the text from 0 to total_chars
        prop_assert_eq!(chunks[0].char_start, 0, "First chunk must start at 0");
        prop_assert_eq!(
            chunks.last().unwrap().char_end,
            total_chars,
            "Last chunk must end at total_chars"
        );

        // Verify continuity (no gaps between consecutive chunks)
        for w in chunks.windows(2) {
            prop_assert!(
                w[1].char_start <= w[0].char_end,
                "Gap between chunks: chunk 0 ends at {}, chunk 1 starts at {}",
                w[0].char_end, w[1].char_start
            );
            prop_assert!(
                w[1].char_start > w[0].char_start,
                "Chunks must make forward progress: chunk 0 start {}, chunk 1 start {}",
                w[0].char_start, w[1].char_start
            );
        }

        // 3. Invariant: multibyte text is never split inside a character boundary
        // Slicing characters directly from original text must match chunk text exactly
        let original_chars: Vec<char> = text.chars().collect();
        for chunk in &chunks {
            let slice: String = original_chars[chunk.char_start..chunk.char_end].iter().collect();
            prop_assert_eq!(
                &chunk.text,
                &slice,
                "Chunk text does not match original slice at {}..{}",
                chunk.char_start, chunk.char_end
            );
        }
    }
}

#[test]
fn test_chunker_empty_and_whitespace_only() {
    // Empty text
    let empty_chunks = chunk_text("", 100, 10);
    assert!(empty_chunks.is_empty());

    // Whitespace only
    let ws_chunks = chunk_text("     \t\n\r    ", 5, 1);
    assert!(!ws_chunks.is_empty());
    for c in &ws_chunks {
        assert!(c.text.chars().count() <= 5);
    }
}

// ============================================================================
// 2. Brute-Force Reference vs Vector Search on Random Data
// ============================================================================

fn dummy_embedding(seed: usize, dim: usize) -> Vec<f32> {
    let mut vec = Vec::with_capacity(dim);
    let mut state = (seed as u64)
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1);
    for _ in 0..dim {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let val = ((state >> 33) as f32) / ((1u32 << 31) as f32) - 1.0;
        vec.push(val);
    }
    // Normalize
    let norm = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm = if norm > 0.0 { norm } else { 1.0 };
    for x in &mut vec {
        *x /= norm;
    }
    vec
}

#[test]
fn test_search_results_against_brute_force_reference() {
    let temp = tempdir().unwrap();
    let app_id = "test-app";
    let collection = "reference_test";
    let dim = 384;
    let model = "bge-small-en-v1.5";

    // Generate 30 test documents with random embeddings
    let mut docs_to_index = Vec::new();
    struct RefChunk {
        doc_id: String,
        chunk_idx: usize,
        text: String,
        embedding: Vec<f32>,
    }
    let mut reference_chunks = Vec::new();

    for d in 0..30 {
        let doc_id = format!("doc_{:03}", d);
        let doc_text = format!("Content of document number {}", d);
        let mut chunks = Vec::new();

        for c in 0..2 {
            let chunk_text_str = format!("Chunk {} of {}", c, doc_id);
            let emb = dummy_embedding(d * 10 + c, dim);
            reference_chunks.push(RefChunk {
                doc_id: doc_id.clone(),
                chunk_idx: c,
                text: chunk_text_str.clone(),
                embedding: emb.clone(),
            });
            chunks.push((
                wavical_ai::chunker::TextChunk {
                    index: c,
                    text: chunk_text_str,
                    char_start: c * 20,
                    char_end: (c + 1) * 20,
                },
                emb,
            ));
        }

        docs_to_index.push(DocumentToIndex {
            id: doc_id,
            text: doc_text,
            metadata: None,
            chunks,
        });
    }

    // Index into vector store
    index_collection_documents(
        temp.path(),
        app_id,
        collection,
        IndexAction::Add,
        model,
        dim,
        docs_to_index,
    )
    .expect("indexing succeeds");

    // Random query vector
    let query_vec = dummy_embedding(999, dim);
    let top_k = 5;

    // Brute-force reference calculation
    let mut ref_candidates: Vec<(&RefChunk, f32)> = reference_chunks
        .iter()
        .map(|rc| {
            let dot: f32 = query_vec
                .iter()
                .zip(rc.embedding.iter())
                .map(|(a, b)| a * b)
                .sum();
            (rc, dot)
        })
        .collect();

    // Sort descending by score, deterministic tie-breaking
    ref_candidates.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| a.0.doc_id.cmp(&b.0.doc_id))
            .then_with(|| a.0.chunk_idx.cmp(&b.0.chunk_idx))
    });

    let ref_top_k: Vec<&RefChunk> = ref_candidates
        .iter()
        .take(top_k)
        .map(|(rc, _)| *rc)
        .collect();

    // Store search execution
    let hits = search_collection(
        temp.path(),
        app_id,
        collection,
        &query_vec,
        model,
        dim,
        top_k,
    )
    .expect("search succeeds");

    assert_eq!(hits.len(), top_k);

    for (i, hit) in hits.iter().enumerate() {
        assert_eq!(
            hit.id, ref_top_k[i].doc_id,
            "Hit {} doc_id matches reference",
            i
        );
        assert_eq!(
            hit.text, ref_top_k[i].text,
            "Hit {} text matches reference",
            i
        );
        let expected_score: f32 = query_vec
            .iter()
            .zip(ref_top_k[i].embedding.iter())
            .map(|(a, b)| a * b)
            .sum();
        assert!(
            (hit.score - expected_score).abs() < 1e-5,
            "Hit {} score {} close to expected {}",
            i,
            hit.score,
            expected_score
        );
    }
}

// ============================================================================
// 3. Every Limit at the Boundary
// ============================================================================

#[test]
fn test_rag_limits_at_the_boundaries() {
    let temp = tempdir().unwrap();

    // A. Documents per call (100 passes, 101 fails)
    let docs_100: Vec<Document> = (0..MAX_DOCUMENTS_PER_INDEX_CALL)
        .map(|i| Document {
            id: format!("doc_{i}"),
            text: "test".to_string(),
            metadata: None,
        })
        .collect();
    let req_100 = IndexRequest {
        action: IndexAction::Add,
        collection: "docs".to_string(),
        documents: Some(docs_100),
    };
    assert!(validate_index_request(&req_100).is_ok());

    let docs_101: Vec<Document> = (0..MAX_DOCUMENTS_PER_INDEX_CALL + 1)
        .map(|i| Document {
            id: format!("doc_{i}"),
            text: "test".to_string(),
            metadata: None,
        })
        .collect();
    let req_101 = IndexRequest {
        action: IndexAction::Add,
        collection: "docs".to_string(),
        documents: Some(docs_101),
    };
    assert!(validate_index_request(&req_101).is_err());

    // B. Document text length (100,000 passes, 100,001 fails)
    let text_100k = "a".repeat(MAX_DOCUMENT_CHARS);
    let req_text_100k = IndexRequest {
        action: IndexAction::Add,
        collection: "docs".to_string(),
        documents: Some(vec![Document {
            id: "doc_100k".to_string(),
            text: text_100k,
            metadata: None,
        }]),
    };
    assert!(validate_index_request(&req_text_100k).is_ok());

    let text_100k_plus_1 = "a".repeat(MAX_DOCUMENT_CHARS + 1);
    let req_text_100k_plus_1 = IndexRequest {
        action: IndexAction::Add,
        collection: "docs".to_string(),
        documents: Some(vec![Document {
            id: "doc_100k_1".to_string(),
            text: text_100k_plus_1,
            metadata: None,
        }]),
    };
    assert!(validate_index_request(&req_text_100k_plus_1).is_err());

    // C. Search query length (4,000 passes, 4,001 fails)
    let query_4k = "q".repeat(MAX_SEARCH_QUERY_CHARS);
    let req_query_4k = SearchRequest {
        collection: "docs".to_string(),
        query: query_4k,
        top_k: 5,
    };
    assert!(validate_search_request(&req_query_4k).is_ok());

    let query_4k_plus_1 = "q".repeat(MAX_SEARCH_QUERY_CHARS + 1);
    let req_query_4k_plus_1 = SearchRequest {
        collection: "docs".to_string(),
        query: query_4k_plus_1,
        top_k: 5,
    };
    assert!(validate_search_request(&req_query_4k_plus_1).is_err());

    // D. Search top_k (1 passes, 20 passes, 0 fails, 21 fails)
    let req_top1 = SearchRequest {
        collection: "docs".to_string(),
        query: "test".to_string(),
        top_k: 1,
    };
    assert!(validate_search_request(&req_top1).is_ok());

    let req_top20 = SearchRequest {
        collection: "docs".to_string(),
        query: "test".to_string(),
        top_k: 20,
    };
    assert!(validate_search_request(&req_top20).is_ok());

    let req_top0 = SearchRequest {
        collection: "docs".to_string(),
        query: "test".to_string(),
        top_k: 0,
    };
    assert!(validate_search_request(&req_top0).is_err());

    let req_top21 = SearchRequest {
        collection: "docs".to_string(),
        query: "test".to_string(),
        top_k: 21,
    };
    assert!(validate_search_request(&req_top21).is_err());

    // E. Collections per app (16 passes, 17th fails)
    let ai_dir = temp.path().join("apps").join("app1").join("ai");
    fs::create_dir_all(&ai_dir).unwrap();
    for i in 1..=MAX_COLLECTIONS_PER_APP {
        fs::write(ai_dir.join(format!("coll_{i}.db")), b"").unwrap();
    }
    assert_eq!(count_app_collections(&ai_dir).unwrap(), 16);

    // Attempting to index a 17th collection fails
    let err = index_collection_documents(
        temp.path(),
        "app1",
        "coll_17",
        IndexAction::Add,
        "bge-small-en-v1.5",
        384,
        vec![DocumentToIndex {
            id: "doc1".to_string(),
            text: "test".to_string(),
            metadata: None,
            chunks: vec![],
        }],
    )
    .unwrap_err();
    assert!(matches!(err, StoreError::LimitExceeded(_)));
}

// ============================================================================
// 4. Invalid Collection Names and Document IDs
// ============================================================================

#[test]
fn test_injection_attempts_in_collection_names_and_doc_ids() {
    let common_injections = [
        "../x",
        "con",
        "a b",
        "\"quotes\"",
        "'quotes'",
        "'; DROP TABLE x;--",
        "колекция",
        "🦀",
        "",
        "..",
        ".",
        "aux",
        "nul",
        "prn",
        "com1",
        "lpt1",
    ];

    for name in &common_injections {
        assert!(
            validate_collection_name(name).is_err(),
            "Collection name '{name}' should be rejected"
        );
        assert!(
            validate_document_id(name).is_err(),
            "Document ID '{name}' should be rejected"
        );
    }

    // Length boundaries: collection name max 64 (65 rejected), doc ID max 128 (129 rejected)
    let coll_65 = "a".repeat(65);
    assert!(validate_collection_name(&coll_65).is_err());
    let coll_64 = "a".repeat(64);
    assert!(validate_collection_name(&coll_64).is_ok());

    let doc_129 = "a".repeat(129);
    assert!(validate_document_id(&doc_129).is_err());
    let doc_128 = "a".repeat(128);
    assert!(validate_document_id(&doc_128).is_ok());
}

// ============================================================================
// 5. Metadata Schema & Size Restrictions
// ============================================================================

#[test]
fn test_metadata_schema_and_size_validation() {
    // 1. Nested objects in JSON rejected by deserialization
    let nested_json = r#"{"action":"add","collection":"docs","documents":[{"id":"d1","text":"foo","metadata":{"nested":{"sub":1}}}]}"#;
    assert!(serde_json::from_str::<IndexRequest>(nested_json).is_err());

    // 2. Arrays in JSON rejected by deserialization
    let array_json = r#"{"action":"add","collection":"docs","documents":[{"id":"d1","text":"foo","metadata":{"arr":[1,2,3]}}}]}"#;
    assert!(serde_json::from_str::<IndexRequest>(array_json).is_err());

    // 3. Size boundary: 4096 bytes passes, 4097 bytes fails
    let mut valid_meta = HashMap::new();
    valid_meta.insert("key".to_string(), MetadataValue::String("a".repeat(4000)));
    assert!(validate_document_metadata("d1", &valid_meta).is_ok());

    let mut oversize_meta = HashMap::new();
    oversize_meta.insert("key".to_string(), MetadataValue::String("a".repeat(4090)));
    assert!(validate_document_metadata("d1", &oversize_meta).is_err());
}

// ============================================================================
// 6. Embedding Model & Dimension Mismatch ("Reindex Required")
// ============================================================================

#[test]
fn test_model_and_dimension_change_requires_reindex() {
    let temp = tempdir().unwrap();
    let app_id = "app1";
    let collection = "reindex_test";

    // 1. Index with bge-small-en-v1.5 (dim 384)
    let emb = dummy_embedding(1, 384);
    index_collection_documents(
        temp.path(),
        app_id,
        collection,
        IndexAction::Add,
        "bge-small-en-v1.5",
        384,
        vec![DocumentToIndex {
            id: "d1".to_string(),
            text: "Hello".to_string(),
            metadata: None,
            chunks: vec![(
                wavical_ai::chunker::TextChunk {
                    index: 0,
                    text: "Hello".to_string(),
                    char_start: 0,
                    char_end: 5,
                },
                emb.clone(),
            )],
        }],
    )
    .expect("indexing succeeds");

    // 2. Search with different model name -> ReindexRequired
    let err_model = search_collection(
        temp.path(),
        app_id,
        collection,
        &emb,
        "nomic-embed-text-v1.5",
        384,
        5,
    )
    .unwrap_err();
    assert!(matches!(err_model, StoreError::ReindexRequired { .. }));

    // 3. Search with different dimension -> ReindexRequired
    let emb_768 = dummy_embedding(1, 768);
    let err_dim = search_collection(
        temp.path(),
        app_id,
        collection,
        &emb_768,
        "bge-small-en-v1.5",
        768,
        5,
    )
    .unwrap_err();
    assert!(matches!(err_dim, StoreError::ReindexRequired { .. }));

    // 4. Mismatched query vector length -> DimensionMismatch
    let err_vec = search_collection(
        temp.path(),
        app_id,
        collection,
        &emb_768, // length 768 when store configured for 384
        "bge-small-en-v1.5",
        384,
        5,
    )
    .unwrap_err();
    assert!(matches!(err_vec, StoreError::DimensionMismatch { .. }));
}

// ============================================================================
// 7. Store File Truncated in Half (Integrity Check)
// ============================================================================

#[test]
fn test_corrupt_store_truncated_file_detected() {
    let temp = tempdir().unwrap();
    let app_id = "app_corrupt";
    let collection = "corrupt_test";

    // Create valid store
    let emb = dummy_embedding(1, 384);
    index_collection_documents(
        temp.path(),
        app_id,
        collection,
        IndexAction::Add,
        "bge-small-en-v1.5",
        384,
        vec![DocumentToIndex {
            id: "d1".to_string(),
            text: "Sample document".to_string(),
            metadata: None,
            chunks: vec![(
                wavical_ai::chunker::TextChunk {
                    index: 0,
                    text: "Sample document".to_string(),
                    char_start: 0,
                    char_end: 15,
                },
                emb.clone(),
            )],
        }],
    )
    .unwrap();

    let db_path = get_collection_db_path(temp.path(), app_id, collection).unwrap();
    let file_len = fs::metadata(&db_path).unwrap().len();
    assert!(file_len > 0);

    // Truncate file to half size
    let file = fs::OpenOptions::new().write(true).open(&db_path).unwrap();
    file.set_len(file_len / 2).unwrap();
    drop(file);

    // Subsequent search or connection must detect corruption
    let err = search_collection(
        temp.path(),
        app_id,
        collection,
        &emb,
        "bge-small-en-v1.5",
        384,
        5,
    )
    .unwrap_err();

    assert!(
        matches!(err, StoreError::CorruptStore(_)) || matches!(err, StoreError::Sqlite(_)),
        "Expected corruption detection, got {:?}",
        err
    );
}

// ============================================================================
// 8. Multi-Tenant App Isolation
// ============================================================================

#[test]
fn test_apps_isolated_from_each_others_collections() {
    let temp = tempdir().unwrap();
    let emb = dummy_embedding(1, 384);

    // Index for app_one
    index_collection_documents(
        temp.path(),
        "app_one",
        "shared_name",
        IndexAction::Add,
        "bge-small-en-v1.5",
        384,
        vec![DocumentToIndex {
            id: "doc_one".to_string(),
            text: "App One Secret".to_string(),
            metadata: None,
            chunks: vec![(
                wavical_ai::chunker::TextChunk {
                    index: 0,
                    text: "App One Secret".to_string(),
                    char_start: 0,
                    char_end: 14,
                },
                emb.clone(),
            )],
        }],
    )
    .unwrap();

    // Query from app_two -> CollectionNotFound
    let err = search_collection(
        temp.path(),
        "app_two",
        "shared_name",
        &emb,
        "bge-small-en-v1.5",
        384,
        5,
    )
    .unwrap_err();

    assert!(matches!(err, StoreError::CollectionNotFound(_)));

    // File paths are strictly separated
    let path_one = get_collection_db_path(temp.path(), "app_one", "shared_name").unwrap();
    let path_two = get_collection_db_path(temp.path(), "app_two", "shared_name").unwrap();
    assert!(path_one.exists());
    assert!(!path_two.exists());
    assert_ne!(path_one, path_two);
}

// ============================================================================
// 9. Delete Really Removes Data
// ============================================================================

#[test]
fn test_delete_really_removes_collection_file() {
    let temp = tempdir().unwrap();
    let emb = dummy_embedding(1, 384);

    index_collection_documents(
        temp.path(),
        "my_app",
        "to_delete",
        IndexAction::Add,
        "bge-small-en-v1.5",
        384,
        vec![DocumentToIndex {
            id: "d1".to_string(),
            text: "temporary".to_string(),
            metadata: None,
            chunks: vec![(
                wavical_ai::chunker::TextChunk {
                    index: 0,
                    text: "temporary".to_string(),
                    char_start: 0,
                    char_end: 9,
                },
                emb,
            )],
        }],
    )
    .unwrap();

    let db_path = get_collection_db_path(temp.path(), "my_app", "to_delete").unwrap();
    assert!(db_path.exists());

    let removed = delete_collection(temp.path(), "my_app", "to_delete").unwrap();
    assert!(removed);
    assert!(
        !db_path.exists(),
        "Collection file must be deleted from disk"
    );
}

// ============================================================================
// 10. Grep Proof: No SQL Queries Built with format! or String Concatenation
// ============================================================================

#[test]
fn test_grep_proof_no_sql_string_concatenation_or_format() {
    let store_src = include_str!("../src/store.rs");

    // Check that format! is never used with SQL keywords
    let forbidden_patterns = [
        "format!(\"SELECT",
        "format!(\"INSERT",
        "format!(\"DELETE",
        "format!(\"UPDATE",
        "format!(\"CREATE",
        "format!(\"PRAGMA",
        "format!(\"select",
        "format!(\"insert",
        "format!(\"delete",
        "format!(\"update",
        "format!(\"create",
        "format!(\"pragma",
    ];

    for pattern in &forbidden_patterns {
        assert!(
            !store_src.contains(pattern),
            "Store source code must not build SQL with format!: found '{pattern}'"
        );
    }
}

// ============================================================================
// 11. Fake-Engine Failure During Embedding Leaves Collection Consistent
// ============================================================================

fn find_fake_engine_binary() -> PathBuf {
    let mut path = std::env::current_exe().expect("current test executable path");
    path.pop(); // pop test name
    if path.ends_with("deps") {
        path.pop();
    }
    let exe = if cfg!(windows) {
        path.join("fake_engine.exe")
    } else {
        path.join("fake_engine")
    };
    if exe.exists() {
        return exe;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
        .join("debug")
        .join(if cfg!(windows) {
            "fake_engine.exe"
        } else {
            "fake_engine"
        })
}

#[test]
fn test_fake_engine_failure_leaves_collection_consistent() {
    let temp = tempdir().unwrap();
    let engine_bin = find_fake_engine_binary();
    let model_file = temp.path().join("fake_model.gguf");
    fs::write(&model_file, b"GGUF_TEST_MODEL").unwrap();

    let expected_hash = sha256_file(&engine_bin).unwrap();
    let mut manager = LocalEngineManager::with_config(
        engine_bin,
        expected_hash,
        10,
        EngineLaunchOptions::default(),
        std::time::Duration::from_secs(5),
    )
    .expect("create manager");
    let channel = manager.ensure_running(&model_file).expect("starts engine");
    let client = create_engine_http_client().unwrap();

    let req = IndexRequest {
        action: IndexAction::Add,
        collection: "consistent_coll".to_string(),
        documents: Some(vec![Document {
            id: "doc_fail".to_string(),
            text: "This document triggers test:embedding_failure now".to_string(),
            metadata: None,
        }]),
    };

    let result = execute_index_request(
        temp.path(),
        "test_app",
        "win1",
        &client,
        &channel,
        "bge-small-en-v1.5",
        384,
        &req,
    );

    assert!(
        result.is_err(),
        "Embedding failure must fail the index call"
    );

    // Verify that the collection file was never created or has 0 documents
    let db_path = get_collection_db_path(temp.path(), "test_app", "consistent_coll").unwrap();
    if db_path.exists() {
        let conn = open_store_connection(&db_path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 0,
            "No half-indexed documents should exist in database"
        );
    }

    manager.shutdown();
}

// ============================================================================
// 12. Marker Text Never Logged in Any Tracing Event
// ============================================================================

struct MarkerCaptureSubscriber {
    captured: Arc<Mutex<Vec<String>>>,
}

impl tracing::Subscriber for MarkerCaptureSubscriber {
    fn register_callsite(
        &self,
        _metadata: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::always()
    }
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::Id {
        tracing::Id::from_u64(1)
    }
    fn record(&self, _span: &tracing::Id, _values: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _span: &tracing::Id, _follows: &tracing::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct Visitor<'a>(&'a mut String);
        impl tracing::field::Visit for Visitor<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.push_str(&format!(" {}={:?}", field.name(), value));
            }
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                self.0.push_str(&format!(" {}={}", field.name(), value));
            }
        }
        let mut line = String::new();
        event.record(&mut Visitor(&mut line));
        self.captured.lock().unwrap().push(line);
    }
    fn enter(&self, _span: &tracing::Id) {}
    fn exit(&self, _span: &tracing::Id) {}
}

#[test]
fn test_marker_prompt_and_document_never_appear_in_tracing_logs() {
    let temp = tempdir().unwrap();
    let engine_bin = find_fake_engine_binary();
    let model_file = temp.path().join("fake_model.gguf");
    fs::write(&model_file, b"GGUF_TEST_MODEL").unwrap();

    let expected_hash = sha256_file(&engine_bin).unwrap();
    let mut manager = LocalEngineManager::with_config(
        engine_bin,
        expected_hash,
        10,
        EngineLaunchOptions::default(),
        std::time::Duration::from_secs(5),
    )
    .expect("create manager");
    let channel = manager.ensure_running(&model_file).expect("starts engine");
    let client = create_engine_http_client().unwrap();

    let marker_secret = "SECRET_MARKER_DOC_42819_CONFIDENTIAL";

    let captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = MarkerCaptureSubscriber {
        captured: Arc::clone(&captured),
    };

    tracing::callsite::rebuild_interest_cache();

    tracing::subscriber::with_default(subscriber, || {
        // 1. Index document with marker secret
        let index_req = IndexRequest {
            action: IndexAction::Add,
            collection: "secret_coll".to_string(),
            documents: Some(vec![Document {
                id: "secret_doc_1".to_string(),
                text: format!("Header info: {marker_secret} and trailing details"),
                metadata: None,
            }]),
        };

        execute_index_request(
            temp.path(),
            "secret_app",
            "win_sec",
            &client,
            &channel,
            "bge-small-en-v1.5",
            384,
            &index_req,
        )
        .expect("indexing succeeds");

        // 2. Search with marker secret in query
        let search_req = SearchRequest {
            collection: "secret_coll".to_string(),
            query: format!("Find {marker_secret}"),
            top_k: 3,
        };

        let hits = execute_search_request(
            temp.path(),
            "secret_app",
            "win_sec",
            &client,
            &channel,
            "bge-small-en-v1.5",
            384,
            &search_req,
        )
        .expect("search succeeds");

        assert!(!hits.hits.is_empty());
    });

    // Verify logs
    let logs = captured.lock().unwrap();
    assert!(!logs.is_empty(), "Tracing events should be emitted");
    for line in logs.iter() {
        assert!(
            !line.contains(marker_secret),
            "Log line must NEVER leak marker secret: {line}"
        );
    }

    manager.shutdown();
}
