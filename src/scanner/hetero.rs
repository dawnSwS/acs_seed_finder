//! GPU-resident exact generation with per-seed CPU recovery for bounded shader
//! failures and complete CPU recovery when a device cannot run the pipeline.

use crate::core::scanner_api::ScanConfig;
#[cfg(not(feature = "gpu"))]
use crate::scanner::cpu;
use crate::scanner::{BackendStatus, GpuKernelCandidate};
use std::sync::{atomic::AtomicBool, atomic::AtomicUsize};

pub(crate) fn scan_seeds_heterogeneous(
    start: i32,
    end: i32,
    config: &ScanConfig,
    progress: &AtomicUsize,
    cancelled: &AtomicBool,
    limit: Option<usize>,
    candidate: Option<GpuKernelCandidate>,
    timestamps: bool,
) -> (
    Vec<(i32, usize)>,
    BackendStatus,
    Option<GpuKernelCandidate>,
    Option<bool>,
) {
    #[cfg(feature = "gpu")]
    {
        gpu::scan(
            (i64::from(start)..=i64::from(end)).filter_map(|seed| i32::try_from(seed).ok()),
            config,
            progress,
            cancelled,
            limit,
            candidate,
            timestamps,
        )
    }
    #[cfg(not(feature = "gpu"))]
    {
        let _ = (candidate, timestamps);
        (
            cpu::scan_seeds_limited(start, end, config, progress, cancelled, limit),
            BackendStatus::CpuFallback("此构建未启用 GPU 功能".to_owned()),
            None,
            None,
        )
    }
}

pub(crate) fn scan_list_heterogeneous(
    list: Vec<i32>,
    config: &ScanConfig,
    progress: &AtomicUsize,
    cancelled: &AtomicBool,
    limit: Option<usize>,
    candidate: Option<GpuKernelCandidate>,
    timestamps: bool,
) -> (
    Vec<(i32, usize)>,
    BackendStatus,
    Option<GpuKernelCandidate>,
    Option<bool>,
) {
    #[cfg(feature = "gpu")]
    {
        gpu::scan(
            list.into_iter(),
            config,
            progress,
            cancelled,
            limit,
            candidate,
            timestamps,
        )
    }
    #[cfg(not(feature = "gpu"))]
    {
        let _ = (candidate, timestamps);
        (
            cpu::scan_seed_list_limited(list, config, progress, cancelled, limit),
            BackendStatus::CpuFallback("此构建未启用 GPU 功能".to_owned()),
            None,
            None,
        )
    }
}

#[cfg(feature = "gpu")]
#[path = "gpu.rs"]
pub(crate) mod gpu;
