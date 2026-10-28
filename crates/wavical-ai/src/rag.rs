#![forbid(unsafe_code)]

use std::path::Path;
use std::time::Instant;

use reqwest::blocking::Client;
use thiserror::Error;

use wavical_ipc::ai::{
    EVENT_AI_CALL_FINISHED, EVENT_AI_CALL_STARTED, IndexAction, IndexRequest, IndexResponse,
    METHOD_AI_INDEX, METHOD_AI_SEARCH, SearchRequest, SearchResponse,
};

use crate::channel::EngineChannel;
use crate::chunker::{DEFAULT_CHUNK_MAX_CHARS, DEFAULT_CHUNK_OVERLAP_CHARS, chunk_text};
use crate::embeddings::{EmbeddingError, compute_embeddings};
use crate::store::{
    DocumentToIndex, StoreError, delete_collection, index_collection_documents, search_collection,
};

/// Errors originating during RAG operations.
#[derive(Debug, Error)]
pub enum RagError {
    #[error("embedding generation failed: {0}")]
    Embedding(#[from] EmbeddingError),

    #[error("vector store operation failed: {0}")]
    Store(#[from] StoreError),

    #[error("reindex required: {0}")]
    ReindexRequired(String),
}

/// Execute an `ai.index` request: add, replace, or delete collection.
///
/// Guarantees:
/// - Atomic: embedding failures or store errors leave the collection unchanged (no half-indexed documents).
/// - Tracing events emit counts, sizes, durations, and IDs only; never document text.
#[allow(clippy::too_many_arguments)]
pub fn execute_index_request(
    app_data_base: &Path,
    app_id: &str,
    window_id: &str,
    client: &Client,
    channel: &EngineChannel,
    embedding_model_id: &str,
    embedding_dimension: usize,
    request: &IndexRequest,
) -> Result<IndexResponse, RagError> {
    let start_time = Instant::now();

    if request.action == IndexAction::Delete {
        tracing::info!(
            event = EVENT_AI_CALL_STARTED,
            window_id = %window_id,
            method = METHOD_AI_INDEX,
            action = "delete",
            collection = %request.collection,
            "AI index delete collection started"
        );

        delete_collection(app_data_base, app_id, &request.collection)?;

        tracing::info!(
            event = EVENT_AI_CALL_FINISHED,
            window_id = %window_id,
            method = METHOD_AI_INDEX,
            action = "delete",
            collection = %request.collection,
            duration_ms = start_time.elapsed().as_millis() as u64,
            "AI index delete collection finished"
        );

        return Ok(IndexResponse {
            ok: true,
            documents_indexed: 0,
            chunks_created: 0,
        });
    }

    let docs = request.documents.as_deref().unwrap_or(&[]);
    let total_input_chars: usize = docs.iter().map(|d| d.text.chars().count()).sum();

    tracing::info!(
        event = EVENT_AI_CALL_STARTED,
        window_id = %window_id,
        method = METHOD_AI_INDEX,
        action = match request.action {
            IndexAction::Add => "add",
            IndexAction::Replace => "replace",
            IndexAction::Delete => "delete",
        },
        collection = %request.collection,
        doc_count = docs.len(),
        input_size_chars = total_input_chars,
        "AI index operation started"
    );

    // 1. Chunk all documents in memory first
    let mut docs_with_chunks = Vec::with_capacity(docs.len());
    let mut all_chunk_texts = Vec::new();

    for doc in docs {
        let chunks = chunk_text(
            &doc.text,
            DEFAULT_CHUNK_MAX_CHARS,
            DEFAULT_CHUNK_OVERLAP_CHARS,
        );
        for c in &chunks {
            all_chunk_texts.push(c.text.clone());
        }
        docs_with_chunks.push((doc, chunks));
    }

    // 2. Compute embeddings for all chunks in a single batched call
    // If the engine fails or crashes here, zero documents are written to SQLite!
    let chunk_refs: Vec<&str> = all_chunk_texts.iter().map(|s| s.as_str()).collect();
    let embeddings = compute_embeddings(client, channel, embedding_model_id, &chunk_refs)?;

    // 3. Assemble DocumentToIndex with matched embeddings
    let mut embedding_iter = embeddings.into_iter();
    let mut docs_to_index = Vec::with_capacity(docs_with_chunks.len());

    for (doc, chunks) in docs_with_chunks {
        let mut chunk_pairs = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            let vec = embedding_iter.next().ok_or_else(|| {
                RagError::Embedding(EmbeddingError::InvalidResponse(
                    "Missing expected embedding for chunk".into(),
                ))
            })?;
            chunk_pairs.push((chunk, vec));
        }

        docs_to_index.push(DocumentToIndex {
            id: doc.id.clone(),
            text: doc.text.clone(),
            metadata: doc.metadata.clone(),
            chunks: chunk_pairs,
        });
    }

    // 4. Atomic transactional write to SQLite store
    let stats = index_collection_documents(
        app_data_base,
        app_id,
        &request.collection,
        request.action,
        embedding_model_id,
        embedding_dimension,
        docs_to_index,
    )?;

    tracing::info!(
        event = EVENT_AI_CALL_FINISHED,
        window_id = %window_id,
        method = METHOD_AI_INDEX,
        collection = %request.collection,
        documents_indexed = stats.documents_indexed,
        chunks_created = stats.chunks_created,
        duration_ms = start_time.elapsed().as_millis() as u64,
        "AI index operation finished successfully"
    );

    Ok(IndexResponse {
        ok: true,
        documents_indexed: stats.documents_indexed,
        chunks_created: stats.chunks_created,
    })
}

/// Execute an `ai.search` request against a collection.
///
/// Guarantees:
/// - Verifies stored embedding model and dimension compatibility.
/// - Never compares vectors of different dimensions.
/// - Returns top `top_k` results ordered deterministically.
/// - Tracing events emit query char count, hit count, durations only; never query text.
#[allow(clippy::too_many_arguments)]
pub fn execute_search_request(
    app_data_base: &Path,
    app_id: &str,
    window_id: &str,
    client: &Client,
    channel: &EngineChannel,
    embedding_model_id: &str,
    embedding_dimension: usize,
    request: &SearchRequest,
) -> Result<SearchResponse, RagError> {
    let start_time = Instant::now();
    let query_chars = request.query.chars().count();

    tracing::info!(
        event = EVENT_AI_CALL_STARTED,
        window_id = %window_id,
        method = METHOD_AI_SEARCH,
        collection = %request.collection,
        input_size_chars = query_chars,
        top_k = request.top_k,
        "AI search operation started"
    );

    // 1. Embed query text
    let query_embeddings =
        compute_embeddings(client, channel, embedding_model_id, &[&request.query])?;
    let query_vec = query_embeddings
        .into_iter()
        .next()
        .ok_or_else(|| RagError::Embedding(EmbeddingError::EmptyVector))?;

    // 2. Search collection with cosine similarity
    let hits = match search_collection(
        app_data_base,
        app_id,
        &request.collection,
        &query_vec,
        embedding_model_id,
        embedding_dimension,
        request.top_k,
    ) {
        Ok(h) => h,
        Err(StoreError::ReindexRequired {
            stored_model,
            stored_dim,
            configured_model,
            configured_dim,
        }) => {
            let msg = format!(
                "reindex required: collection indexed with model '{stored_model}' (dim {stored_dim}), current configuration specifies '{configured_model}' (dim {configured_dim})"
            );
            return Err(RagError::ReindexRequired(msg));
        }
        Err(e) => return Err(RagError::Store(e)),
    };

    tracing::info!(
        event = EVENT_AI_CALL_FINISHED,
        window_id = %window_id,
        method = METHOD_AI_SEARCH,
        collection = %request.collection,
        hit_count = hits.len(),
        duration_ms = start_time.elapsed().as_millis() as u64,
        "AI search operation finished successfully"
    );

    Ok(SearchResponse { hits })
}
