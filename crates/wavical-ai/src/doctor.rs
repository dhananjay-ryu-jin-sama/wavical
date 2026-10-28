#![forbid(unsafe_code)]

//! System diagnostics and hardware suitability report for `wavical ai doctor`.
//!
//! Reports:
//! - Total RAM and whether it satisfies the 8 GB target.
//! - CPU SIMD instruction extensions that impact CPU inference performance (AVX2, FMA, AVX, SSE4.2).
//! - Engine binary presence and SHA-256 verification status.
//! - Installed and configured catalog models.
//! - Memory footprint suitability for 8 GB RAM machines.

use std::fmt::Write as _;
use std::path::Path;
use sysinfo::System;

use crate::catalog::default_catalog;
use crate::commands::list_models;
use crate::engine::{expected_engine_hash, resolve_engine_path, verify_engine_binary};
use crate::storage::wavical_models_dir;

/// Diagnostic summary evaluated by `wavical ai doctor`.
#[derive(Debug, Clone)]
pub struct DoctorReport {
    pub total_ram_bytes: u64,
    pub total_ram_gb: f64,
    pub meets_8gb_target: bool,
    pub has_avx2: bool,
    pub has_fma: bool,
    pub has_avx: bool,
    pub has_sse42: bool,
    pub has_avx512f: bool,
    pub engine_path: String,
    pub engine_present: bool,
    pub engine_hash_valid: bool,
    pub engine_hash_error: Option<String>,
    pub installed_models: Vec<DoctorModelInfo>,
}

#[derive(Debug, Clone)]
pub struct DoctorModelInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub installed: bool,
    pub size_mb: f64,
    pub min_ram_gb: f64,
    pub fits_in_target_ram: bool,
}

/// Run system diagnostics and generate a `DoctorReport`.
pub fn run_ai_doctor(models_dir: Option<&Path>) -> DoctorReport {
    // 1. RAM detection
    let mut sys = System::new();
    sys.refresh_memory();
    let total_ram_bytes = sys.total_memory();
    let total_ram_gb = total_ram_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    // Allow down to 7.0 GB to account for integrated GPU hardware reservation on 8 GB systems
    let meets_8gb_target = total_ram_gb >= 7.0;

    // 2. CPU SIMD extensions
    #[cfg(target_arch = "x86_64")]
    let (has_avx2, has_fma, has_avx, has_sse42, has_avx512f) = (
        std::is_x86_feature_detected!("avx2"),
        std::is_x86_feature_detected!("fma"),
        std::is_x86_feature_detected!("avx"),
        std::is_x86_feature_detected!("sse4.2"),
        std::is_x86_feature_detected!("avx512f"),
    );
    #[cfg(not(target_arch = "x86_64"))]
    let (has_avx2, has_fma, has_avx, has_sse42, has_avx512f) = (false, false, false, false, false);

    // 3. Engine binary presence and hash check
    let engine_path = resolve_engine_path();
    let expected_hash = expected_engine_hash();
    let engine_present = engine_path.is_file();
    let (engine_hash_valid, engine_hash_error) = if engine_present {
        match verify_engine_binary(&engine_path, &expected_hash) {
            Ok(()) => (true, None),
            Err(e) => (false, Some(e.to_string())),
        }
    } else {
        (false, None)
    };

    // 4. Installed models inspection
    let m_dir = match models_dir {
        Some(d) => d.to_path_buf(),
        None => wavical_models_dir(),
    };
    let catalog = default_catalog();
    let model_statuses = list_models(&m_dir, Some(catalog));

    let mut installed_models = Vec::new();
    for status in model_statuses {
        let entry = catalog.find_by_id(&status.id);
        let min_ram_gb = entry
            .map(|e| e.min_ram_bytes as f64 / (1024.0 * 1024.0 * 1024.0))
            .unwrap_or(2.0);
        let fits = min_ram_gb <= 4.0; // Comfortably fits in 8 GB with system headroom

        installed_models.push(DoctorModelInfo {
            id: status.id,
            name: status.name,
            kind: format!("{:?}", status.kind).to_lowercase(),
            installed: status.installed,
            size_mb: status.size_bytes as f64 / (1024.0 * 1024.0),
            min_ram_gb,
            fits_in_target_ram: fits,
        });
    }

    DoctorReport {
        total_ram_bytes,
        total_ram_gb,
        meets_8gb_target,
        has_avx2,
        has_fma,
        has_avx,
        has_sse42,
        has_avx512f,
        engine_path: engine_path.display().to_string(),
        engine_present,
        engine_hash_valid,
        engine_hash_error,
        installed_models,
    }
}

/// Format the `DoctorReport` into human-friendly CLI output.
pub fn format_doctor_report(report: &DoctorReport) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "Wavical AI System Doctor");
    let _ = writeln!(out, "{}", "=".repeat(64));

    // System Memory
    let ram_status = if report.meets_8gb_target {
        "✓ PASS (Target: 8 GB)"
    } else {
        "⚠ WARN (Below 8 GB target; large models may experience swapping)"
    };
    let _ = writeln!(out, "\n[System Memory]");
    let _ = writeln!(out, "  Total RAM:          {:.2} GB", report.total_ram_gb);
    let _ = writeln!(out, "  Target Assessment:  {}", ram_status);

    // CPU Acceleration
    let _ = writeln!(out, "\n[CPU Acceleration Features]");
    let _ = writeln!(
        out,
        "  AVX2 (High Perf):   {}",
        if report.has_avx2 {
            "✓ Supported"
        } else {
            "✗ Not detected"
        }
    );
    let _ = writeln!(
        out,
        "  FMA:                {}",
        if report.has_fma {
            "✓ Supported"
        } else {
            "✗ Not detected"
        }
    );
    let _ = writeln!(
        out,
        "  AVX:                {}",
        if report.has_avx {
            "✓ Supported"
        } else {
            "✗ Not detected"
        }
    );
    let _ = writeln!(
        out,
        "  SSE4.2 (Baseline):  {}",
        if report.has_sse42 {
            "✓ Supported"
        } else {
            "✗ Not detected"
        }
    );
    let _ = writeln!(
        out,
        "  AVX-512:            {}",
        if report.has_avx512f {
            "✓ Supported"
        } else {
            "- Not detected (optional)"
        }
    );

    if report.has_avx2 {
        let _ = writeln!(
            out,
            "  Inference Kernel:   Optimal (CPU AVX2 SIMD acceleration available)"
        );
    } else {
        let _ = writeln!(
            out,
            "  Inference Kernel:   Fallback (AVX2 missing; inference will use standard CPU kernels)"
        );
    }

    // Engine Runtime
    let _ = writeln!(out, "\n[Engine Runtime]");
    let _ = writeln!(out, "  Binary Path:        {}", report.engine_path);
    if !report.engine_present {
        let _ = writeln!(
            out,
            "  Status:             ✗ Missing (Run 'wavical model pull <id>' or install llama-server)"
        );
    } else if report.engine_hash_valid {
        let _ = writeln!(
            out,
            "  Status:             ✓ Present & SHA-256 Digest Verified"
        );
    } else {
        let _ = writeln!(
            out,
            "  Status:             ✗ FAILED (SHA-256 mismatch: {})",
            report
                .engine_hash_error
                .as_deref()
                .unwrap_or("digest altered")
        );
    }

    // Models
    let _ = writeln!(out, "\n[Catalog Models]");
    for m in &report.installed_models {
        let inst_mark = if m.installed {
            "✓ Installed"
        } else {
            "○ Not installed"
        };
        let ram_fit = if m.fits_in_target_ram {
            "✓ Fits 8 GB"
        } else {
            "⚠ Exceeds 8 GB target"
        };
        let _ = writeln!(
            out,
            "  - {:<22} [{:<9}] {:<14} ({:>6.1} MB, Min RAM: {:.1} GB - {})",
            m.id, m.kind, inst_mark, m.size_mb, m.min_ram_gb, ram_fit
        );
    }

    let _ = writeln!(out, "\n{}", "=".repeat(64));
    out
}
