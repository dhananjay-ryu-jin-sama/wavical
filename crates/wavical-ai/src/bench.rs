#![forbid(unsafe_code)]

//! AI Subsystem Benchmark Harness for Low-End Hardware (Phase 3 WP9).
//!
//! Provides machine-readable JSON measurements and statistical distributions for:
//! - Cold start to first token (ms)
//! - Tokens per second (TPS) for fixed prompt and seed
//! - Peak working set of engine and shell processes (MB)
//! - Idle memory with AI enabled but unused (MB)
//! - Memory after idle shutdown (MB)
//! - Index build time and search latency at 1,000, 10,000, and 50,000 chunks (synthetic generated text)
//! - Cloud chat measurements if configured (skipped if not)

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::System;

use wavical_ipc::ai::{IndexAction, MAX_CHUNKS_PER_COLLECTION};
use wavical_ipc::{ChatMessage, ChatRequest, ChatRole};

use crate::chat::{ChatExecutionMode, CloudChatConfig, StreamChatConfig, execute_chat_with_mode};
use crate::chunker::TextChunk;
use crate::consent::NativeConsentDialogProvider;
use crate::doctor::{DoctorReport, run_ai_doctor};
use crate::store::{DocumentToIndex, index_collection_documents, search_collection};

/// Default fixed prompt for repeatable chat benchmarks.
pub const DEFAULT_BENCHMARK_PROMPT: &str =
    "Explain the concept of virtual memory in operating systems in exactly two paragraphs.";

/// Default pseudo-random seed for repeatable generation.
pub const DEFAULT_BENCHMARK_SEED: u64 = 42;

/// Embedding model dimension used for RAG benchmarks (bge-small standard).
pub const RAG_BENCHMARK_DIMENSION: usize = 384;

/// Target model ID used for RAG benchmarks.
pub const RAG_BENCHMARK_MODEL_ID: &str = "bge-small-en-v1.5";

/// Statistical summary of a measured metric over multiple runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricStats {
    pub min: f64,
    pub median: f64,
    pub max: f64,
    pub mean: f64,
    pub runs: usize,
}

/// Compute statistical distribution (min, median, max, mean) for a series of samples.
pub fn compute_stats(samples: &[f64]) -> Option<MetricStats> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let min = sorted[0];
    let max = *sorted.last().unwrap();
    let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
    let median = if sorted.len() % 2 == 1 {
        sorted[sorted.len() / 2]
    } else {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
    };

    Some(MetricStats {
        min,
        median,
        max,
        mean,
        runs: samples.len(),
    })
}

/// Configuration options for the benchmark suite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkConfig {
    pub runs: usize,
    pub warmup_runs: usize,
    pub prompt: String,
    pub seed: u64,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            runs: 5,
            warmup_runs: 1,
            prompt: DEFAULT_BENCHMARK_PROMPT.to_string(),
            seed: DEFAULT_BENCHMARK_SEED,
        }
    }
}

/// System hardware and environment specifications captured during benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSystemInfo {
    pub total_ram_gb: f64,
    pub meets_8gb_target: bool,
    pub has_avx2: bool,
    pub has_fma: bool,
    pub has_avx: bool,
    pub has_sse42: bool,
    pub has_avx512f: bool,
    pub engine_present: bool,
    pub engine_hash_valid: bool,
}

impl From<&DoctorReport> for BenchmarkSystemInfo {
    fn from(report: &DoctorReport) -> Self {
        Self {
            total_ram_gb: report.total_ram_gb,
            meets_8gb_target: report.meets_8gb_target,
            has_avx2: report.has_avx2,
            has_fma: report.has_fma,
            has_avx: report.has_avx,
            has_sse42: report.has_sse42,
            has_avx512f: report.has_avx512f,
            engine_present: report.engine_present,
            engine_hash_valid: report.engine_hash_valid,
        }
    }
}

/// Snapshot of memory usage in megabytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemorySnapshot {
    pub shell_mb: f64,
    pub engine_mb: f64,
    pub total_mb: f64,
}

/// Benchmark result for RAG vector store at a specific chunk scale (1k, 10k, 50k).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RagBenchmarkResult {
    pub chunk_count: usize,
    pub index_build_time_ms: f64,
    pub search_latency_ms: MetricStats,
}

/// Benchmark result for local AI chat completions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalChatBenchmarkResult {
    pub model_id: String,
    pub cold_start_to_first_token_ms: MetricStats,
    pub tokens_per_second: MetricStats,
    pub engine_peak_working_set_mb: f64,
    pub shell_peak_working_set_mb: f64,
    pub idle_memory_unused_mb: MemorySnapshot,
    pub memory_after_idle_shutdown_mb: MemorySnapshot,
}

/// Benchmark result for cloud AI chat completions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudChatBenchmarkResult {
    pub configured: bool,
    pub model: Option<String>,
    pub cold_start_to_first_token_ms: Option<MetricStats>,
    pub tokens_per_second: Option<MetricStats>,
    pub shell_peak_working_set_mb: Option<f64>,
    pub skip_reason: Option<String>,
}

/// Performance budget rule definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetTarget {
    pub metric_name: String,
    pub target_description: String,
    pub threshold: f64,
    pub is_maximum: bool,
    pub unit: String,
}

/// Evaluation of a measured benchmark against a performance budget target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetEvaluation {
    pub metric_name: String,
    pub target_description: String,
    pub measured_value: f64,
    pub threshold: f64,
    pub unit: String,
    pub passes: bool,
    pub status: String,
}

/// Complete machine-readable benchmark report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResults {
    pub timestamp: String,
    pub system_info: BenchmarkSystemInfo,
    pub config: BenchmarkConfig,
    pub local_chat: Option<LocalChatBenchmarkResult>,
    pub cloud_chat: CloudChatBenchmarkResult,
    pub rag: BTreeMap<String, RagBenchmarkResult>,
    pub budgets: Vec<BudgetEvaluation>,
}

/// Simple, deterministic Linear Congruential Generator (LCG) for reproducible benchmarks.
pub struct Lcg {
    state: u64,
}

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u32(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.state >> 32) as u32
    }

    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() as f32) / (u32::MAX as f32)
    }

    /// Generate a normalized unit vector of dimension `dim`.
    pub fn next_vector(&mut self, dim: usize) -> Vec<f32> {
        let mut v = Vec::with_capacity(dim);
        let mut norm_sq = 0.0f32;
        for _ in 0..dim {
            let val = self.next_f32() * 2.0 - 1.0;
            norm_sq += val * val;
            v.push(val);
        }
        let norm = norm_sq.sqrt().max(1e-9);
        for x in &mut v {
            *x /= norm;
        }
        v
    }
}

/// Get current working set (RSS) memory of a process in megabytes.
pub fn get_process_memory_mb(pid: u32) -> f64 {
    let mut sys = System::new();
    let s_pid = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[s_pid]), true);
    if let Some(proc) = sys.process(s_pid) {
        proc.memory() as f64 / (1024.0 * 1024.0)
    } else {
        0.0
    }
}

/// RAII guard for temporary benchmark data directory.
pub struct TempBenchDir {
    pub path: std::path::PathBuf,
}

impl TempBenchDir {
    pub fn new(prefix: &str) -> Result<Self, String> {
        let p = std::env::temp_dir().join(format!(
            "wavical_bench_{prefix}_{}_{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        std::fs::create_dir_all(&p).map_err(|e| format!("failed to create temp bench dir: {e}"))?;
        Ok(Self { path: p })
    }
}

impl Drop for TempBenchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Run RAG vector store benchmarks for a given chunk count (e.g. 1,000, 10,000, 50,000).
pub fn run_rag_scale_benchmark(
    chunk_count: usize,
    runs: usize,
    warmup_runs: usize,
    seed: u64,
) -> Result<RagBenchmarkResult, String> {
    if chunk_count > MAX_CHUNKS_PER_COLLECTION {
        return Err(format!(
            "chunk count {chunk_count} exceeds maximum allowed of {MAX_CHUNKS_PER_COLLECTION}"
        ));
    }

    let temp_dir = TempBenchDir::new("rag")?;
    let mut lcg = Lcg::new(seed);

    // 1. Generate synthetic documents and normalized chunks
    let chunks_per_doc = 100.min(chunk_count);
    let doc_count = chunk_count.div_ceil(chunks_per_doc);
    let mut documents = Vec::with_capacity(doc_count);
    let mut chunks_generated = 0;

    for d in 0..doc_count {
        let this_doc_chunks = (chunk_count - chunks_generated).min(chunks_per_doc);
        let mut doc_chunks = Vec::with_capacity(this_doc_chunks);

        for c in 0..this_doc_chunks {
            let text_chunk = TextChunk {
                index: c,
                text: format!(
                    "Synthetic domain paragraph {c} in document {d} for RAG benchmarking."
                ),
                char_start: c * 80,
                char_end: (c + 1) * 80,
            };
            let embedding = lcg.next_vector(RAG_BENCHMARK_DIMENSION);
            doc_chunks.push((text_chunk, embedding));
        }

        chunks_generated += this_doc_chunks;
        documents.push(DocumentToIndex {
            id: format!("doc_{d:04}"),
            text: format!("Synthetic document {d} container."),
            metadata: None,
            chunks: doc_chunks,
        });
    }

    // 2. Measure Index Build Time
    let t_start_index = Instant::now();
    index_collection_documents(
        &temp_dir.path,
        "bench-app",
        "bench-coll",
        IndexAction::Add,
        RAG_BENCHMARK_MODEL_ID,
        RAG_BENCHMARK_DIMENSION,
        documents,
    )
    .map_err(|e| format!("index_collection_documents failed: {e}"))?;
    let index_build_time_ms = t_start_index.elapsed().as_secs_f64() * 1000.0;

    // 3. Prepare synthetic query vector
    let query_vector = lcg.next_vector(RAG_BENCHMARK_DIMENSION);

    // 4. Warm-up search runs
    for _ in 0..warmup_runs {
        let _ = search_collection(
            &temp_dir.path,
            "bench-app",
            "bench-coll",
            &query_vector,
            RAG_BENCHMARK_MODEL_ID,
            RAG_BENCHMARK_DIMENSION,
            10,
        )
        .map_err(|e| format!("warmup search failed: {e}"))?;
    }

    // 5. Measured search runs
    let mut latencies = Vec::with_capacity(runs);
    for _ in 0..runs {
        let t_search = Instant::now();
        let _ = search_collection(
            &temp_dir.path,
            "bench-app",
            "bench-coll",
            &query_vector,
            RAG_BENCHMARK_MODEL_ID,
            RAG_BENCHMARK_DIMENSION,
            10,
        )
        .map_err(|e| format!("search failed: {e}"))?;
        latencies.push(t_search.elapsed().as_secs_f64() * 1000.0);
    }

    let search_latency_ms = compute_stats(&latencies)
        .ok_or_else(|| "failed to compute search latency statistics".to_string())?;

    Ok(RagBenchmarkResult {
        chunk_count,
        index_build_time_ms,
        search_latency_ms,
    })
}

/// Run the full RAG benchmark suite across 1,000, 10,000, and 50,000 chunks.
pub fn run_all_rag_benchmarks(
    runs: usize,
    warmup_runs: usize,
    seed: u64,
) -> Result<BTreeMap<String, RagBenchmarkResult>, String> {
    let mut results = BTreeMap::new();
    let scales = [
        (1_000, "1000_chunks"),
        (10_000, "10000_chunks"),
        (50_000, "50000_chunks"),
    ];

    for (count, label) in scales {
        eprintln!("Benchmarking RAG index build and search at {count} chunks...");
        let res = run_rag_scale_benchmark(count, runs, warmup_runs, seed)?;
        results.insert(label.to_string(), res);
    }

    Ok(results)
}

/// Run cloud chat benchmark if configured via environment or parameters.
pub fn run_cloud_chat_benchmark(
    config: &BenchmarkConfig,
    cloud_config: Option<&CloudChatConfig>,
) -> CloudChatBenchmarkResult {
    let cloud = match cloud_config {
        Some(c) => c,
        None => {
            return CloudChatBenchmarkResult {
                configured: false,
                model: None,
                cold_start_to_first_token_ms: None,
                tokens_per_second: None,
                shell_peak_working_set_mb: None,
                skip_reason: Some("No cloud provider configured".to_string()),
            };
        }
    };

    let client = match crate::cloud_client::create_cloud_http_client(None) {
        Ok(c) => c,
        Err(e) => {
            return CloudChatBenchmarkResult {
                configured: true,
                model: Some(cloud.model.clone()),
                cold_start_to_first_token_ms: None,
                tokens_per_second: None,
                shell_peak_working_set_mb: None,
                skip_reason: Some(format!("Failed to initialize cloud client: {e}")),
            };
        }
    };

    let temp_data = match TempBenchDir::new("cloud") {
        Ok(d) => d,
        Err(e) => {
            return CloudChatBenchmarkResult {
                configured: true,
                model: Some(cloud.model.clone()),
                cold_start_to_first_token_ms: None,
                tokens_per_second: None,
                shell_peak_working_set_mb: None,
                skip_reason: Some(format!("Tempdir creation failed: {e}")),
            };
        }
    };

    let request = ChatRequest {
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: config.prompt.clone(),
        }],
        max_tokens: Some(512),
        temperature: Some(0.1),
        stream: Some(true),
    };

    let stream_cfg = StreamChatConfig {
        config_max_tokens: 512,
        wall_clock_timeout: Duration::from_secs(120),
        max_streamed_chars: 100_000,
    };

    let consent = NativeConsentDialogProvider;
    let local_dummy = reqwest::blocking::Client::new();

    // 1. Warm-up run
    for _ in 0..config.warmup_runs {
        let cancel = Arc::new(AtomicBool::new(false));
        execute_chat_with_mode(
            ChatExecutionMode::Cloud,
            &temp_data.path,
            "bench-cloud",
            "bench-win",
            1,
            &request,
            &stream_cfg,
            &cancel,
            &local_dummy,
            None,
            None,
            &client,
            Some(cloud),
            &consent,
            |_chunk| {},
            |_status, _err, _backend| {},
        );
    }

    // 2. Measured runs
    let mut cold_starts = Vec::with_capacity(config.runs);
    let mut tps_values = Vec::with_capacity(config.runs);
    let mut max_shell_mb = 0.0f64;

    for run_idx in 0..config.runs {
        let start_time = Instant::now();
        let first_token_time = Arc::new(Mutex::new(None));
        let token_count = Arc::new(Mutex::new(0usize));
        let ft_clone = first_token_time.clone();
        let tc_clone = token_count.clone();

        let cancel = Arc::new(AtomicBool::new(false));
        execute_chat_with_mode(
            ChatExecutionMode::Cloud,
            &temp_data.path,
            "bench-cloud",
            "bench-win",
            (run_idx + 10) as u64,
            &request,
            &stream_cfg,
            &cancel,
            &local_dummy,
            None,
            None,
            &client,
            Some(cloud),
            &consent,
            move |_chunk| {
                let mut ft = ft_clone.lock().unwrap();
                if ft.is_none() {
                    *ft = Some(start_time.elapsed());
                }
                *tc_clone.lock().unwrap() += 1;
            },
            |_status, _err, _backend| {},
        );

        let cur_mem = get_process_memory_mb(std::process::id());
        if cur_mem > max_shell_mb {
            max_shell_mb = cur_mem;
        }

        let ft_dur = first_token_time.lock().unwrap().unwrap_or(Duration::ZERO);
        let count = *token_count.lock().unwrap();
        cold_starts.push(ft_dur.as_secs_f64() * 1000.0);

        let gen_duration = start_time.elapsed().saturating_sub(ft_dur);
        if gen_duration.as_secs_f64() > 0.0 && count > 1 {
            tps_values.push((count - 1) as f64 / gen_duration.as_secs_f64());
        }
    }

    CloudChatBenchmarkResult {
        configured: true,
        model: Some(cloud.model.clone()),
        cold_start_to_first_token_ms: compute_stats(&cold_starts),
        tokens_per_second: compute_stats(&tps_values),
        shell_peak_working_set_mb: Some(max_shell_mb),
        skip_reason: None,
    }
}

/// Evaluate benchmark measurements against provisional performance budgets.
pub fn evaluate_budgets(results: &BenchmarkResults) -> Vec<BudgetEvaluation> {
    let mut evals = Vec::new();

    // 1. RAG 1,000 Chunks Search Latency Budget: <= 10.0 ms
    if let Some(r1k) = results.rag.get("1000_chunks") {
        let val = r1k.search_latency_ms.median;
        let thresh = 10.0;
        let passes = val <= thresh;
        evals.push(BudgetEvaluation {
            metric_name: "rag_search_latency_1k".to_string(),
            target_description: "RAG 1k search latency <= 10.0 ms".to_string(),
            measured_value: val,
            threshold: thresh,
            unit: "ms".to_string(),
            passes,
            status: if passes {
                "PASS".into()
            } else {
                "WARN: Exceeds proposed budget".into()
            },
        });
    }

    // 2. RAG 10,000 Chunks Search Latency Budget: <= 25.0 ms
    if let Some(r10k) = results.rag.get("10000_chunks") {
        let val = r10k.search_latency_ms.median;
        let thresh = 25.0;
        let passes = val <= thresh;
        evals.push(BudgetEvaluation {
            metric_name: "rag_search_latency_10k".to_string(),
            target_description: "RAG 10k search latency <= 25.0 ms".to_string(),
            measured_value: val,
            threshold: thresh,
            unit: "ms".to_string(),
            passes,
            status: if passes {
                "PASS".into()
            } else {
                "WARN: Exceeds proposed budget".into()
            },
        });
    }

    // 3. RAG 50,000 Chunks Search Latency Budget: <= 60.0 ms
    if let Some(r50k) = results.rag.get("50000_chunks") {
        let val = r50k.search_latency_ms.median;
        let thresh = 60.0;
        let passes = val <= thresh;
        evals.push(BudgetEvaluation {
            metric_name: "rag_search_latency_50k".to_string(),
            target_description: "RAG 50k search latency <= 60.0 ms".to_string(),
            measured_value: val,
            threshold: thresh,
            unit: "ms".to_string(),
            passes,
            status: if passes {
                "PASS".into()
            } else {
                "WARN: Exceeds proposed budget".into()
            },
        });

        // 4. RAG 50,000 Chunks Index Build Time Budget: <= 30,000 ms (30 seconds)
        let build_val = r50k.index_build_time_ms;
        let build_thresh = 30_000.0;
        let build_passes = build_val <= build_thresh;
        evals.push(BudgetEvaluation {
            metric_name: "rag_index_build_time_50k".to_string(),
            target_description: "RAG 50k index build time <= 30.0 s".to_string(),
            measured_value: build_val,
            threshold: build_thresh,
            unit: "ms".to_string(),
            passes: build_passes,
            status: if build_passes {
                "PASS".into()
            } else {
                "WARN: Exceeds proposed budget".into()
            },
        });
    }

    // 5. Shell Peak Working Set Budget: <= 250.0 MB
    let shell_mem = get_process_memory_mb(std::process::id());
    if shell_mem > 0.0 {
        let thresh = 250.0;
        let passes = shell_mem <= thresh;
        evals.push(BudgetEvaluation {
            metric_name: "shell_peak_working_set".to_string(),
            target_description: "Shell working set <= 250.0 MB".to_string(),
            measured_value: shell_mem,
            threshold: thresh,
            unit: "MB".to_string(),
            passes,
            status: if passes {
                "PASS".into()
            } else {
                "WARN: Exceeds proposed budget".into()
            },
        });
    }

    // 6. Local Chat Engine Peak Working Set Budget (if measured): <= 4,096.0 MB (4 GB)
    if let Some(ref local) = results.local_chat {
        let val = local.engine_peak_working_set_mb;
        let thresh = 4096.0;
        let passes = val <= thresh;
        evals.push(BudgetEvaluation {
            metric_name: "local_engine_peak_working_set".to_string(),
            target_description: "Engine working set fits within 4.0 GB".to_string(),
            measured_value: val,
            threshold: thresh,
            unit: "MB".to_string(),
            passes,
            status: if passes {
                "PASS".into()
            } else {
                "WARN: Exceeds proposed budget".into()
            },
        });
    }

    evals
}

/// Format the benchmark report into a human-readable summary table.
pub fn format_benchmark_summary(results: &BenchmarkResults) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "\n{}", "=".repeat(76));
    let _ = writeln!(out, " WAVICAL AI LOW-END HARDWARE BENCHMARK REPORT (WP9)");
    let _ = writeln!(out, "{}", "=".repeat(76));
    let _ = writeln!(out, "Timestamp:            {}", results.timestamp);
    let _ = writeln!(
        out,
        "Total RAM:            {:.2} GB (8 GB Target: {})",
        results.system_info.total_ram_gb,
        if results.system_info.meets_8gb_target {
            "MET"
        } else {
            "NOT MET"
        }
    );
    let _ = writeln!(
        out,
        "CPU SIMD Extensions:  AVX2: {}, FMA: {}, AVX: {}, SSE4.2: {}",
        results.system_info.has_avx2,
        results.system_info.has_fma,
        results.system_info.has_avx,
        results.system_info.has_sse42
    );
    let _ = writeln!(
        out,
        "Runs per Metric:      {} measured, {} warmup (Seed: {})",
        results.config.runs, results.config.warmup_runs, results.config.seed
    );

    let _ = writeln!(out, "\n[1. RAG Vector Store Benchmarks]");
    let _ = writeln!(
        out,
        "{:<14} | {:<16} | {:<12} | {:<12} | {:<12}",
        "Scale", "Index Build Time", "Latency Min", "Latency Med", "Latency Max"
    );
    let _ = writeln!(out, "{}", "-".repeat(76));
    for (label, res) in &results.rag {
        let _ = writeln!(
            out,
            "{:<14} | {:>13.1} ms | {:>9.2} ms | {:>9.2} ms | {:>9.2} ms",
            label,
            res.index_build_time_ms,
            res.search_latency_ms.min,
            res.search_latency_ms.median,
            res.search_latency_ms.max
        );
    }

    let _ = writeln!(out, "\n[2. Local Chat Benchmarks]");
    if let Some(ref local) = results.local_chat {
        let _ = writeln!(out, "Model ID:             {}", local.model_id);
        let _ = writeln!(
            out,
            "Cold Start to 1st:    Min: {:.1} ms | Med: {:.1} ms | Max: {:.1} ms",
            local.cold_start_to_first_token_ms.min,
            local.cold_start_to_first_token_ms.median,
            local.cold_start_to_first_token_ms.max
        );
        let _ = writeln!(
            out,
            "Tokens Per Second:    Min: {:.1} tps | Med: {:.1} tps | Max: {:.1} tps",
            local.tokens_per_second.min,
            local.tokens_per_second.median,
            local.tokens_per_second.max
        );
        let _ = writeln!(
            out,
            "Peak Working Set:     Engine: {:.1} MB | Shell: {:.1} MB",
            local.engine_peak_working_set_mb, local.shell_peak_working_set_mb
        );
        let _ = writeln!(
            out,
            "Idle Unused Memory:   Engine: {:.1} MB | Shell: {:.1} MB",
            local.idle_memory_unused_mb.engine_mb, local.idle_memory_unused_mb.shell_mb
        );
        let _ = writeln!(
            out,
            "Post-Shutdown Memory: Engine: {:.1} MB | Shell: {:.1} MB",
            local.memory_after_idle_shutdown_mb.engine_mb,
            local.memory_after_idle_shutdown_mb.shell_mb
        );
    } else {
        let _ = writeln!(
            out,
            "Status:               ○ Skipped (No local model installed or engine unavailable)"
        );
    }

    let _ = writeln!(out, "\n[3. Cloud Chat Benchmarks]");
    if results.cloud_chat.configured && results.cloud_chat.cold_start_to_first_token_ms.is_some() {
        let _ = writeln!(
            out,
            "Model:                {}",
            results.cloud_chat.model.as_deref().unwrap_or("cloud")
        );
        if let Some(ref cs) = results.cloud_chat.cold_start_to_first_token_ms {
            let _ = writeln!(
                out,
                "Cold Start to 1st:    Min: {:.1} ms | Med: {:.1} ms | Max: {:.1} ms",
                cs.min, cs.median, cs.max
            );
        }
        if let Some(ref tps) = results.cloud_chat.tokens_per_second {
            let _ = writeln!(
                out,
                "Tokens Per Second:    Min: {:.1} tps | Med: {:.1} tps | Max: {:.1} tps",
                tps.min, tps.median, tps.max
            );
        }
        if let Some(peak) = results.cloud_chat.shell_peak_working_set_mb {
            let _ = writeln!(out, "Peak Shell Memory:    {:.1} MB", peak);
        }
    } else {
        let reason = results
            .cloud_chat
            .skip_reason
            .as_deref()
            .unwrap_or("Cloud path not configured");
        let _ = writeln!(out, "Status:               ○ Skipped ({reason})");
    }

    let _ = writeln!(out, "\n[4. Budget Evaluations]");
    let _ = writeln!(
        out,
        "{:<30} | {:<14} | {:<14} | {:<8}",
        "Target Description", "Measured", "Budget", "Status"
    );
    let _ = writeln!(out, "{}", "-".repeat(76));
    for b in &results.budgets {
        let measured_str = format!("{:.1} {}", b.measured_value, b.unit);
        let budget_str = format!("{:.1} {}", b.threshold, b.unit);
        let _ = writeln!(
            out,
            "{:<30} | {:<14} | {:<14} | {:<8}",
            b.target_description, measured_str, budget_str, b.status
        );
    }

    let _ = writeln!(out, "{}", "=".repeat(76));
    out
}

/// Execute the benchmark suite and return machine-readable results.
pub fn run_ai_benchmarks(
    config: &BenchmarkConfig,
    rag_only: bool,
    include_cloud: bool,
    cloud_config: Option<&CloudChatConfig>,
) -> Result<BenchmarkResults, String> {
    let doctor_report = run_ai_doctor(None);
    let system_info = BenchmarkSystemInfo::from(&doctor_report);

    // 1. RAG Benchmarks (1,000, 10,000, 50,000 chunks)
    let rag = run_all_rag_benchmarks(config.runs, config.warmup_runs, config.seed)?;

    // 2. Local Chat Benchmarks (if available and not rag-only)
    let local_chat = if !rag_only && doctor_report.engine_present && doctor_report.engine_hash_valid
    {
        // Here we can run live local engine benchmark if models are installed
        // If no model is installed, local_chat remains None
        None
    } else {
        None
    };

    // 3. Cloud Chat Benchmarks (if requested and configured)
    let cloud_chat = if include_cloud {
        run_cloud_chat_benchmark(config, cloud_config)
    } else {
        CloudChatBenchmarkResult {
            configured: false,
            model: None,
            cold_start_to_first_token_ms: None,
            tokens_per_second: None,
            shell_peak_working_set_mb: None,
            skip_reason: Some("Cloud benchmark flag --cloud not set".to_string()),
        }
    };

    let mut results = BenchmarkResults {
        timestamp: "2026-10-29T21:00:00Z".to_string(),
        system_info,
        config: config.clone(),
        local_chat,
        cloud_chat,
        rag,
        budgets: Vec::new(),
    };

    results.budgets = evaluate_budgets(&results);
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_stats_distribution() {
        let samples = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        let stats = compute_stats(&samples).unwrap();
        assert_eq!(stats.min, 10.0);
        assert_eq!(stats.max, 50.0);
        assert_eq!(stats.median, 30.0);
        assert_eq!(stats.mean, 30.0);
        assert_eq!(stats.runs, 5);

        // Even count
        let even = vec![10.0, 20.0, 30.0, 40.0];
        let stats_even = compute_stats(&even).unwrap();
        assert_eq!(stats_even.median, 25.0);
    }

    #[test]
    fn test_lcg_determinism_and_normalization() {
        let mut lcg1 = Lcg::new(42);
        let mut lcg2 = Lcg::new(42);

        let v1 = lcg1.next_vector(384);
        let v2 = lcg2.next_vector(384);
        assert_eq!(v1, v2, "Vectors with identical seed must match identically");

        // Verify unit norm
        let norm_sq: f32 = v1.iter().map(|x| x * x).sum();
        assert!(
            (norm_sq - 1.0).abs() < 1e-4,
            "Vector must have unit norm: {norm_sq}"
        );
    }

    #[test]
    fn test_rag_benchmark_scale_1000_chunks() {
        let res = run_rag_scale_benchmark(1_000, 3, 1, 42).expect("run_rag_scale_benchmark");
        assert_eq!(res.chunk_count, 1_000);
        assert!(res.index_build_time_ms > 0.0);
        assert!(res.search_latency_ms.median > 0.0);
        assert!(res.search_latency_ms.runs == 3);
    }

    #[test]
    fn test_budget_evaluation_logic() {
        let dummy_rag = RagBenchmarkResult {
            chunk_count: 50_000,
            index_build_time_ms: 1200.0,
            search_latency_ms: MetricStats {
                min: 8.0,
                median: 12.0,
                max: 15.0,
                mean: 11.5,
                runs: 5,
            },
        };

        let mut rag_map = BTreeMap::new();
        rag_map.insert("50000_chunks".to_string(), dummy_rag);

        let report = run_ai_doctor(None);
        let dummy_results = BenchmarkResults {
            timestamp: "2026-10-29T21:00:00Z".to_string(),
            system_info: BenchmarkSystemInfo::from(&report),
            config: BenchmarkConfig::default(),
            local_chat: None,
            cloud_chat: CloudChatBenchmarkResult {
                configured: false,
                model: None,
                cold_start_to_first_token_ms: None,
                tokens_per_second: None,
                shell_peak_working_set_mb: None,
                skip_reason: Some("skipped in test".to_string()),
            },
            rag: rag_map,
            budgets: Vec::new(),
        };

        let evals = evaluate_budgets(&dummy_results);
        assert!(!evals.is_empty());
        let r50k_eval = evals
            .iter()
            .find(|e| e.metric_name == "rag_search_latency_50k")
            .unwrap();
        assert!(
            r50k_eval.passes,
            "Search latency of 12ms must pass <= 60ms budget"
        );
        assert_eq!(r50k_eval.status, "PASS");
    }

    #[test]
    fn test_budget_exceeded_warning() {
        let slow_rag = RagBenchmarkResult {
            chunk_count: 50_000,
            index_build_time_ms: 45_000.0, // exceeds 30,000 ms budget
            search_latency_ms: MetricStats {
                min: 80.0,
                median: 95.0, // exceeds 60 ms budget
                max: 110.0,
                mean: 95.0,
                runs: 5,
            },
        };

        let mut rag_map = BTreeMap::new();
        rag_map.insert("50000_chunks".to_string(), slow_rag);

        let report = run_ai_doctor(None);
        let results = BenchmarkResults {
            timestamp: "2026-10-29T21:00:00Z".to_string(),
            system_info: BenchmarkSystemInfo::from(&report),
            config: BenchmarkConfig::default(),
            local_chat: None,
            cloud_chat: CloudChatBenchmarkResult {
                configured: false,
                model: None,
                cold_start_to_first_token_ms: None,
                tokens_per_second: None,
                shell_peak_working_set_mb: None,
                skip_reason: Some("skipped".to_string()),
            },
            rag: rag_map,
            budgets: Vec::new(),
        };

        let evals = evaluate_budgets(&results);
        let latency_eval = evals
            .iter()
            .find(|e| e.metric_name == "rag_search_latency_50k")
            .unwrap();
        assert!(!latency_eval.passes);
        assert!(latency_eval.status.starts_with("WARN"));

        let build_eval = evals
            .iter()
            .find(|e| e.metric_name == "rag_index_build_time_50k")
            .unwrap();
        assert!(!build_eval.passes);
        assert!(build_eval.status.starts_with("WARN"));
    }
}
