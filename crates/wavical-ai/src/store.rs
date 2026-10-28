#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::{Connection, params};
use thiserror::Error;

use wavical_ipc::ai::{
    IndexAction, MAX_CHUNKS_PER_COLLECTION, MAX_COLLECTIONS_PER_APP, MAX_INDEX_FILE_SIZE_BYTES,
    MetadataValue, SearchHit, is_reserved_name, validate_collection_name,
};

use crate::chunker::TextChunk;

// ============================================================================
// Parameterized SQL Statement Constants (Grep Proof: Zero format! / concatenation)
// ============================================================================

const SQL_PRAGMA_QUICK_CHECK: &str = "PRAGMA quick_check;";
const SQL_PRAGMA_FOREIGN_KEYS: &str = "PRAGMA foreign_keys = ON;";

const SQL_CREATE_META_TABLE: &str =
    "CREATE TABLE IF NOT EXISTS collection_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);";

const SQL_CREATE_DOCS_TABLE: &str = "CREATE TABLE IF NOT EXISTS documents (id TEXT PRIMARY KEY, text TEXT NOT NULL, metadata_json TEXT, created_at INTEGER NOT NULL);";

const SQL_CREATE_CHUNKS_TABLE: &str = "CREATE TABLE IF NOT EXISTS chunks (id INTEGER PRIMARY KEY AUTOINCREMENT, doc_id TEXT NOT NULL, chunk_index INTEGER NOT NULL, text TEXT NOT NULL, char_start INTEGER NOT NULL, char_end INTEGER NOT NULL, embedding BLOB NOT NULL, FOREIGN KEY(doc_id) REFERENCES documents(id) ON DELETE CASCADE);";

const SQL_CREATE_CHUNKS_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_chunks_doc_id ON chunks(doc_id);";

const SQL_SELECT_META: &str = "SELECT value FROM collection_meta WHERE key = ?1;";

const SQL_INSERT_META: &str =
    "INSERT OR REPLACE INTO collection_meta (key, value) VALUES (?1, ?2);";

const SQL_COUNT_CHUNKS: &str = "SELECT COUNT(*) FROM chunks;";

const SQL_COUNT_CHUNKS_BY_DOC: &str = "SELECT COUNT(*) FROM chunks WHERE doc_id = ?1;";

const SQL_DELETE_DOC: &str = "DELETE FROM documents WHERE id = ?1;";

const SQL_DELETE_CHUNKS_BY_DOC: &str = "DELETE FROM chunks WHERE doc_id = ?1;";

const SQL_INSERT_DOC: &str = "INSERT OR REPLACE INTO documents (id, text, metadata_json, created_at) VALUES (?1, ?2, ?3, ?4);";

const SQL_INSERT_CHUNK: &str = "INSERT INTO chunks (doc_id, chunk_index, text, char_start, char_end, embedding) VALUES (?1, ?2, ?3, ?4, ?5, ?6);";

const SQL_SELECT_ALL_CHUNKS: &str = "SELECT c.doc_id, c.chunk_index, c.text, c.embedding, d.metadata_json FROM chunks c JOIN documents d ON c.doc_id = d.id;";

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("invalid appId '{0}'")]
    InvalidAppId(String),

    #[error("invalid collection name '{0}'")]
    InvalidCollection(String),

    #[error("collection '{0}' does not exist")]
    CollectionNotFound(String),

    #[error(
        "reindex required: collection indexed with model '{stored_model}' (dim {stored_dim}), current configuration specifies '{configured_model}' (dim {configured_dim})"
    )]
    ReindexRequired {
        stored_model: String,
        stored_dim: usize,
        configured_model: String,
        configured_dim: usize,
    },

    #[error("vector dimension mismatch: expected {expected}, received {got}")]
    DimensionMismatch { expected: usize, got: usize },

    #[error("store file corruption detected: {0}")]
    CorruptStore(String),

    #[error("index limit exceeded: {0}")]
    LimitExceeded(String),

    #[error("database I/O or SQLite execution error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),

    #[error("metadata serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

// ============================================================================
// Store Path Resolution & Sandboxing
// ============================================================================

/// Validate an application identifier.
///
/// Restricted to `[A-Za-z0-9._-]{1,64}` and cannot match Windows reserved device names.
pub fn validate_app_id(app_id: &str) -> Result<(), StoreError> {
    if app_id.is_empty() || app_id.len() > 64 {
        return Err(StoreError::InvalidAppId(app_id.to_string()));
    }
    if is_reserved_name(app_id) {
        return Err(StoreError::InvalidAppId(app_id.to_string()));
    }
    for c in app_id.chars() {
        let is_valid = c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-';
        if !is_valid {
            return Err(StoreError::InvalidAppId(app_id.to_string()));
        }
    }
    Ok(())
}

/// Compute the isolated SQLite database path for an app's collection.
///
/// Path structure: `<app_data_base>/apps/<app_id>/ai/<collection>.db`
pub fn get_collection_db_path(
    app_data_base: &Path,
    app_id: &str,
    collection: &str,
) -> Result<PathBuf, StoreError> {
    validate_app_id(app_id)?;
    validate_collection_name(collection)
        .map_err(|_| StoreError::InvalidCollection(collection.to_string()))?;

    let dir = app_data_base.join("apps").join(app_id).join("ai");
    let file_name = format!("{collection}.db");
    Ok(dir.join(file_name))
}

/// Return the directory holding all collection stores for a given app.
pub fn get_app_ai_dir(app_data_base: &Path, app_id: &str) -> Result<PathBuf, StoreError> {
    validate_app_id(app_id)?;
    Ok(app_data_base.join("apps").join(app_id).join("ai"))
}

// ============================================================================
// Limits Enforcement
// ============================================================================

/// Calculate total index byte size across all `.db` and journal files for an app.
pub fn compute_app_index_size(ai_dir: &Path) -> Result<u64, StoreError> {
    if !ai_dir.exists() {
        return Ok(0);
    }
    let mut total: u64 = 0;
    for entry in fs::read_dir(ai_dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

/// Count existing collections for an app.
pub fn count_app_collections(ai_dir: &Path) -> Result<usize, StoreError> {
    if !ai_dir.exists() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in fs::read_dir(ai_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("db") {
            count += 1;
        }
    }
    Ok(count)
}

// ============================================================================
// Vector Serialization & Pure Cosine Similarity
// ============================================================================

/// Serialize an `f32` slice into a raw little-endian byte vector.
pub fn embedding_to_bytes(vec: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vec.len() * 4);
    for &val in vec {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// Deserialize raw little-endian bytes back into an `f32` vector.
pub fn bytes_to_embedding(bytes: &[u8]) -> Result<Vec<f32>, StoreError> {
    if !bytes.len().is_multiple_of(4) {
        return Err(StoreError::CorruptStore(
            "Embedding blob length is not a multiple of 4 bytes".into(),
        ));
    }
    let (chunks, _) = bytes.as_chunks::<4>();
    let mut vec = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        vec.push(f32::from_le_bytes(*chunk));
    }
    Ok(vec)
}

/// Compute cosine similarity between two equal-length vectors in pure Rust.
///
/// Formula: dot(u, v) / (norm(u) * norm(v))
/// Returns None if dimensions do not match or vectors are empty.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    if norm_a <= 0.0 || norm_b <= 0.0 {
        return Some(0.0);
    }

    Some(dot / (norm_a.sqrt() * norm_b.sqrt()))
}

// ============================================================================
// SQLite Connection & Integrity Validation
// ============================================================================

/// Open an SQLite connection with PRAGMA quick_check integrity verification.
///
/// Detects corrupt or truncated database files immediately upon opening.
pub fn open_store_connection(path: &Path) -> Result<Connection, StoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let conn = Connection::open(path)?;

    // Enable foreign keys
    conn.execute(SQL_PRAGMA_FOREIGN_KEYS, [])?;

    // If database already exists and has content, verify integrity
    if path.exists() && fs::metadata(path)?.len() > 0 {
        let mut stmt = conn.prepare(SQL_PRAGMA_QUICK_CHECK)?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let status: String = row.get(0)?;
            if status != "ok" {
                return Err(StoreError::CorruptStore(format!(
                    "SQLite integrity check failed: {status}"
                )));
            }
        }
    }

    // Initialize required schema
    conn.execute(SQL_CREATE_META_TABLE, [])?;
    conn.execute(SQL_CREATE_DOCS_TABLE, [])?;
    conn.execute(SQL_CREATE_CHUNKS_TABLE, [])?;
    conn.execute(SQL_CREATE_CHUNKS_INDEX, [])?;

    Ok(conn)
}

// ============================================================================
// Storage Operations (ai.index & ai.search)
// ============================================================================

/// Document payload to be stored with its embedded chunks.
pub struct DocumentToIndex {
    pub id: String,
    pub text: String,
    pub metadata: Option<HashMap<String, MetadataValue>>,
    pub chunks: Vec<(TextChunk, Vec<f32>)>,
}

/// Statistics returned after indexing documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexResultStats {
    pub documents_indexed: usize,
    pub chunks_created: usize,
}

/// Index documents into a collection store.
///
/// Guarantees:
/// - Atomic write: executes inside a database transaction; rollback on error.
/// - Limits enforcement: collections per app (<=16), total size (<=512 MB), chunks per collection (<=50,000).
/// - Saves `embedding_model_id` and `vector_dimension` metadata.
/// - Parameterized queries only.
pub fn index_collection_documents(
    app_data_base: &Path,
    app_id: &str,
    collection: &str,
    action: IndexAction,
    model_id: &str,
    dimension: usize,
    documents: Vec<DocumentToIndex>,
) -> Result<IndexResultStats, StoreError> {
    let ai_dir = get_app_ai_dir(app_data_base, app_id)?;
    let db_path = get_collection_db_path(app_data_base, app_id, collection)?;

    // 1. Check collection count limit (16 collections per app)
    let is_new_collection = !db_path.exists();
    if is_new_collection {
        let coll_count = count_app_collections(&ai_dir)?;
        if coll_count >= MAX_COLLECTIONS_PER_APP {
            return Err(StoreError::LimitExceeded(format!(
                "App exceeds maximum collections limit of {MAX_COLLECTIONS_PER_APP} (currently {coll_count})"
            )));
        }
    }

    // 2. Check total app index file size limit (512 MB)
    let current_app_size = compute_app_index_size(&ai_dir)?;
    if current_app_size >= MAX_INDEX_FILE_SIZE_BYTES {
        return Err(StoreError::LimitExceeded(format!(
            "App exceeds maximum index size limit of {MAX_INDEX_FILE_SIZE_BYTES} bytes"
        )));
    }

    let mut conn = open_store_connection(&db_path)?;
    let tx = conn.transaction()?;

    // 3. Verify or record model and dimension metadata
    let stored_model: Option<String> = {
        let mut stmt = tx.prepare(SQL_SELECT_META)?;
        let mut rows = stmt.query(params!["embedding_model_id"])?;
        if let Some(row) = rows.next()? {
            Some(row.get(0)?)
        } else {
            None
        }
    };

    let stored_dim: Option<usize> = {
        let mut stmt = tx.prepare(SQL_SELECT_META)?;
        let mut rows = stmt.query(params!["vector_dimension"])?;
        if let Some(row) = rows.next()? {
            let dim_str: String = row.get(0)?;
            dim_str.parse().ok()
        } else {
            None
        }
    };

    if let (Some(m), Some(d)) = (stored_model, stored_dim) {
        if m != model_id || d != dimension {
            return Err(StoreError::ReindexRequired {
                stored_model: m,
                stored_dim: d,
                configured_model: model_id.to_string(),
                configured_dim: dimension,
            });
        }
    } else {
        tx.execute(SQL_INSERT_META, params!["embedding_model_id", model_id])?;
        tx.execute(
            SQL_INSERT_META,
            params!["vector_dimension", dimension.to_string()],
        )?;
    }

    // 4. Check chunk capacity limit (50,000 chunks per collection)
    let current_chunks: usize = {
        let mut stmt = tx.prepare(SQL_COUNT_CHUNKS)?;
        let count: i64 = stmt.query_row([], |row| row.get(0))?;
        count.max(0) as usize
    };

    let new_chunks_count: usize = documents.iter().map(|d| d.chunks.len()).sum();

    // If replacing documents, chunks of those existing documents will be removed
    let mut chunks_to_remove = 0;
    if action == IndexAction::Replace {
        for doc in &documents {
            let mut stmt = tx.prepare(SQL_COUNT_CHUNKS_BY_DOC)?;
            let count: i64 = stmt.query_row(params![doc.id], |row| row.get(0))?;
            chunks_to_remove += count.max(0) as usize;
        }
    }

    let projected_chunks = current_chunks
        .saturating_sub(chunks_to_remove)
        .saturating_add(new_chunks_count);

    if projected_chunks > MAX_CHUNKS_PER_COLLECTION {
        return Err(StoreError::LimitExceeded(format!(
            "Collection exceeds maximum chunks limit of {MAX_CHUNKS_PER_COLLECTION} (projected {projected_chunks})"
        )));
    }

    // 5. Insert documents and chunks
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    for doc in &documents {
        if action == IndexAction::Replace {
            tx.execute(SQL_DELETE_CHUNKS_BY_DOC, params![doc.id])?;
            tx.execute(SQL_DELETE_DOC, params![doc.id])?;
        }

        let metadata_json = match &doc.metadata {
            Some(m) => Some(serde_json::to_string(m)?),
            None => None,
        };

        tx.execute(
            SQL_INSERT_DOC,
            params![doc.id, doc.text, metadata_json, now],
        )?;

        for (chunk, embedding) in &doc.chunks {
            if embedding.len() != dimension {
                return Err(StoreError::DimensionMismatch {
                    expected: dimension,
                    got: embedding.len(),
                });
            }
            let blob = embedding_to_bytes(embedding);
            tx.execute(
                SQL_INSERT_CHUNK,
                params![
                    doc.id,
                    chunk.index as i64,
                    chunk.text,
                    chunk.char_start as i64,
                    chunk.char_end as i64,
                    blob,
                ],
            )?;
        }
    }

    tx.commit()?;

    Ok(IndexResultStats {
        documents_indexed: documents.len(),
        chunks_created: new_chunks_count,
    })
}

/// Delete a collection store and remove its database file and WAL sidecars from disk.
pub fn delete_collection(
    app_data_base: &Path,
    app_id: &str,
    collection: &str,
) -> Result<bool, StoreError> {
    let db_path = get_collection_db_path(app_data_base, app_id, collection)?;
    if !db_path.exists() {
        return Ok(false);
    }

    // Remove main database file
    fs::remove_file(&db_path)?;

    // Remove any WAL and SHM journal sidecars if present
    let wal = db_path.with_extension("db-wal");
    if wal.exists() {
        let _ = fs::remove_file(wal);
    }
    let shm = db_path.with_extension("db-shm");
    if shm.exists() {
        let _ = fs::remove_file(shm);
    }

    Ok(true)
}

/// Search a collection for the top-k most similar chunks.
///
/// Guarantees:
/// - Validates that stored embedding model and dimension match configuration.
/// - Never compares vectors of different dimensions.
/// - Deterministic tie-breaking for equal scores (by doc_id ascending, then chunk_index ascending).
/// - Returns top `top_k` results.
pub fn search_collection(
    app_data_base: &Path,
    app_id: &str,
    collection: &str,
    query_embedding: &[f32],
    configured_model_id: &str,
    configured_dimension: usize,
    top_k: usize,
) -> Result<Vec<SearchHit>, StoreError> {
    let db_path = get_collection_db_path(app_data_base, app_id, collection)?;
    if !db_path.exists() {
        return Err(StoreError::CollectionNotFound(collection.to_string()));
    }

    let conn = open_store_connection(&db_path)?;

    // 1. Verify embedding model and dimension compatibility
    let stored_model: String = {
        let mut stmt = conn.prepare(SQL_SELECT_META)?;
        stmt.query_row(params!["embedding_model_id"], |row| row.get(0))
            .map_err(|_| {
                StoreError::CorruptStore("Missing embedding_model_id in metadata".into())
            })?
    };

    let stored_dim: usize = {
        let mut stmt = conn.prepare(SQL_SELECT_META)?;
        let dim_str: String = stmt
            .query_row(params!["vector_dimension"], |row| row.get(0))
            .map_err(|_| StoreError::CorruptStore("Missing vector_dimension in metadata".into()))?;
        dim_str
            .parse()
            .map_err(|_| StoreError::CorruptStore("Invalid vector_dimension in metadata".into()))?
    };

    if stored_model != configured_model_id || stored_dim != configured_dimension {
        return Err(StoreError::ReindexRequired {
            stored_model,
            stored_dim,
            configured_model: configured_model_id.to_string(),
            configured_dim: configured_dimension,
        });
    }

    if query_embedding.len() != stored_dim {
        return Err(StoreError::DimensionMismatch {
            expected: stored_dim,
            got: query_embedding.len(),
        });
    }

    // 2. Scan all chunks, compute cosine similarity
    struct Candidate {
        doc_id: String,
        chunk_index: i64,
        text: String,
        metadata: Option<HashMap<String, MetadataValue>>,
        score: f32,
    }

    let mut stmt = conn.prepare(SQL_SELECT_ALL_CHUNKS)?;
    let mut rows = stmt.query([])?;
    let mut candidates: Vec<Candidate> = Vec::new();

    while let Some(row) = rows.next()? {
        let doc_id: String = row.get(0)?;
        let chunk_index: i64 = row.get(1)?;
        let text: String = row.get(2)?;
        let blob: Vec<u8> = row.get(3)?;
        let meta_json: Option<String> = row.get(4)?;

        let embedding = bytes_to_embedding(&blob)?;
        let score = cosine_similarity(query_embedding, &embedding).unwrap_or(0.0);

        let metadata = match meta_json {
            Some(s) => serde_json::from_str(&s).ok(),
            None => None,
        };

        candidates.push(Candidate {
            doc_id,
            chunk_index,
            text,
            metadata,
            score,
        });
    }

    // 3. Sort descending by score, deterministic tie-breaking by doc_id then chunk_index
    candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.doc_id.cmp(&b.doc_id))
            .then_with(|| a.chunk_index.cmp(&b.chunk_index))
    });

    let top_results = candidates
        .into_iter()
        .take(top_k)
        .map(|c| SearchHit {
            id: c.doc_id,
            score: c.score,
            text: c.text,
            metadata: c.metadata,
        })
        .collect();

    Ok(top_results)
}
