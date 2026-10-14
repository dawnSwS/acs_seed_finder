mod cpu;
mod hetero;

#[cfg(feature = "gpu")]
pub use hetero::gpu::{
    begin_gui_diagnostics, debug_diagnostics_snapshot, end_gui_diagnostics,
    set_gui_diagnostic_run_id,
};

use crate::core::scanner_api::{ComputeMode, ScanConfig};
use std::fmt;
use std::sync::{atomic::AtomicBool, atomic::AtomicUsize, Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuKernelCandidate {
    V3,
    Cooperative,
    CooperativeWords,
}
impl GpuKernelCandidate {
    pub fn name(self) -> &'static str {
        match self {
            Self::V3 => "v3",
            Self::Cooperative => "cooperative",
            Self::CooperativeWords => "cooperative_words",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendStatus {
    Cpu,
    /// Every committed seed was generated and counted by the GPU.
    #[cfg(feature = "gpu")]
    Gpu,
    /// Explicitly enabled software GPU execution, never a hardware-GPU result.
    #[cfg(feature = "gpu")]
    SoftwareGpu {
        cpu_fallback_seeds: usize,
        reason: String,
    },
    /// Some seeds required exact CPU recomputation; successful GPU results remain.
    #[cfg(feature = "gpu")]
    GpuWithCpuFallback {
        seeds: usize,
        reason: String,
    },
    CpuFallback(String),
    InvalidConfig(String),
}

impl fmt::Display for BackendStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cpu => formatter.write_str("CPU 并行扫描"),
            #[cfg(feature = "gpu")]
            Self::Gpu => formatter.write_str("GPU 完整生成与计数"),
            #[cfg(feature = "gpu")]
            Self::SoftwareGpu {
                cpu_fallback_seeds,
                reason,
            } => {
                if *cpu_fallback_seeds == 0 {
                    write!(formatter, "软件 GPU 完整生成与计数（非硬件 GPU）：{reason}")
                } else {
                    write!(
                        formatter,
                        "软件 GPU + CPU 回退 {cpu_fallback_seeds} 个种子：{reason}"
                    )
                }
            }
            #[cfg(feature = "gpu")]
            Self::GpuWithCpuFallback { seeds, reason } => {
                write!(formatter, "GPU 生成 + CPU 回退 {seeds} 个种子：{reason}")
            }
            Self::CpuFallback(reason) => write!(formatter, "CPU 回退：{reason}"),
            Self::InvalidConfig(reason) => write!(formatter, "扫描配置无效：{reason}"),
        }
    }
}

pub struct EngineScanner {
    pub mode: ComputeMode,
    backend: Mutex<BackendStatus>,
    gpu_candidate: Option<GpuKernelCandidate>,
    actual_gpu_candidate: Mutex<Option<GpuKernelCandidate>>,
    gpu_timestamps: bool,
    gpu_timestamp_config_error: Option<String>,
    actual_gpu_timestamps: Mutex<Option<bool>>,
}
impl EngineScanner {
    pub fn new(mode: ComputeMode) -> Self {
        let environment = std::env::var("ACS_GPU_TIMESTAMPS");
        let parsed = match environment.as_ref() {
            Ok(value) => timestamp_setting(Some(value)),
            Err(std::env::VarError::NotPresent) => timestamp_setting(None),
            Err(std::env::VarError::NotUnicode(_)) => Err("ACS_GPU_TIMESTAMPS 必须为 0 或 1"),
        };
        let (gpu_timestamps, gpu_timestamp_config_error) = match parsed {
            Ok(enabled) => (enabled, None),
            Err(reason) => (true, Some(reason.to_owned())),
        };
        Self {
            mode,
            backend: Mutex::new(BackendStatus::Cpu),
            gpu_candidate: None,
            actual_gpu_candidate: Mutex::new(None),
            gpu_timestamps,
            gpu_timestamp_config_error,
            actual_gpu_timestamps: Mutex::new(None),
        }
    }

    /// Select a candidate for this scanner without mutating process environment.
    pub fn new_gpu_candidate(candidate: GpuKernelCandidate) -> Self {
        let mut scanner = Self::new(ComputeMode::HeterogeneousPipeline);
        scanner.gpu_candidate = Some(candidate);
        // Explicit benchmark candidates do not inherit an unrelated process
        // diagnostic environment; the builder below controls each A/B row.
        scanner.gpu_timestamps = true;
        scanner.gpu_timestamp_config_error = None;
        scanner
    }

    /// Diagnostic A/B option. This changes query instrumentation, never the
    /// shader, GPU context, generated seeds, or count/filter/sort semantics.
    pub fn with_gpu_timestamps(mut self, enabled: bool) -> Self {
        self.gpu_timestamps = enabled;
        self.gpu_timestamp_config_error = None;
        self
    }

    pub fn requested_gpu_timestamps(&self) -> bool {
        self.gpu_timestamps
    }

    /// Whether the last scan's GPU buffers actually enabled timestamp queries.
    /// None means no GPU scan buffers were obtained. Inspect backend_status
    /// separately: this field does not claim successful GPU completion.
    pub fn actual_gpu_timestamps(&self) -> Option<bool> {
        *self
            .actual_gpu_timestamps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn set_actual_gpu_timestamps(&self, enabled: Option<bool>) {
        *self
            .actual_gpu_timestamps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = enabled;
    }

    /// Identity of the ready GPU context attempted by the last scan, if any.
    /// A nonempty value does not imply success: inspect backend_status as well.
    pub fn actual_gpu_kernel(&self) -> Option<&'static str> {
        self.actual_gpu_candidate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .map(GpuKernelCandidate::name)
    }

    fn set_actual_gpu_candidate(&self, candidate: Option<GpuKernelCandidate>) {
        *self
            .actual_gpu_candidate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = candidate;
    }

    pub fn backend_status(&self) -> BackendStatus {
        self.backend
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn set_backend(&self, status: BackendStatus) {
        *self
            .backend
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = status;
    }

    fn valid_config(&self, config: &ScanConfig) -> bool {
        // An invalid GPU diagnostic option must not affect the CPU oracle.
        if matches!(self.mode, ComputeMode::HeterogeneousPipeline) {
            if let Some(error) = &self.gpu_timestamp_config_error {
                self.set_backend(BackendStatus::InvalidConfig(error.clone()));
                return false;
            }
        }
        match config.validate() {
            Ok(()) => true,
            Err(error) => {
                self.set_backend(BackendStatus::InvalidConfig(error.to_string()));
                false
            }
        }
    }

    /// Scan all seeds while retaining at most `limit` matches in each worker.
    #[allow(clippy::needless_pass_by_value)]
    pub fn scan_top_cancellable(
        &self,
        start: i32,
        end: i32,
        config: &ScanConfig,
        limit: usize,
        progress: Arc<AtomicUsize>,
        cancelled: Arc<AtomicBool>,
    ) -> Vec<(i32, usize)> {
        self.scan_with_limit(start, end, config, Some(limit), &progress, &cancelled)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn scan_list_top_cancellable(
        &self,
        list: Vec<i32>,
        config: &ScanConfig,
        limit: usize,
        progress: Arc<AtomicUsize>,
        cancelled: Arc<AtomicBool>,
    ) -> Vec<(i32, usize)> {
        self.scan_list_with_limit(list, config, Some(limit), &progress, &cancelled)
    }

    fn scan_list_with_limit(
        &self,
        list: Vec<i32>,
        config: &ScanConfig,
        limit: Option<usize>,
        progress: &AtomicUsize,
        cancelled: &AtomicBool,
    ) -> Vec<(i32, usize)> {
        self.set_actual_gpu_candidate(None);
        self.set_actual_gpu_timestamps(None);
        if !self.valid_config(config) {
            return Vec::new();
        }
        match self.mode {
            ComputeMode::CpuRayon => {
                self.set_backend(BackendStatus::Cpu);
                cpu::scan_seed_list_limited(list, config, progress, cancelled, limit)
            }
            ComputeMode::HeterogeneousPipeline => {
                let (matches, status, actual_candidate, actual_timestamps) =
                    hetero::scan_list_heterogeneous(
                        list,
                        config,
                        progress,
                        cancelled,
                        limit,
                        self.gpu_candidate,
                        self.gpu_timestamps,
                    );
                self.set_actual_gpu_candidate(actual_candidate);
                self.set_actual_gpu_timestamps(actual_timestamps);
                self.set_backend(status);
                matches
            }
        }
    }

    fn scan_with_limit(
        &self,
        start: i32,
        end: i32,
        config: &ScanConfig,
        limit: Option<usize>,
        progress: &AtomicUsize,
        cancelled: &AtomicBool,
    ) -> Vec<(i32, usize)> {
        self.set_actual_gpu_candidate(None);
        self.set_actual_gpu_timestamps(None);
        if !self.valid_config(config) {
            return Vec::new();
        }
        match self.mode {
            ComputeMode::CpuRayon => {
                self.set_backend(BackendStatus::Cpu);
                cpu::scan_seeds_limited(start, end, config, progress, cancelled, limit)
            }
            ComputeMode::HeterogeneousPipeline => {
                let (matches, status, actual_candidate, actual_timestamps) =
                    hetero::scan_seeds_heterogeneous(
                        start,
                        end,
                        config,
                        progress,
                        cancelled,
                        limit,
                        self.gpu_candidate,
                        self.gpu_timestamps,
                    );
                self.set_actual_gpu_candidate(actual_candidate);
                self.set_actual_gpu_timestamps(actual_timestamps);
                self.set_backend(status);
                matches
            }
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn scan_cancellable(
        &self,
        start: i32,
        end: i32,
        config: &ScanConfig,
        progress: Arc<AtomicUsize>,
        cancelled: Arc<AtomicBool>,
    ) -> Vec<(i32, usize)> {
        self.scan_with_limit(start, end, config, None, &progress, &cancelled)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn scan_list_cancellable(
        &self,
        list: Vec<i32>,
        config: &ScanConfig,
        progress: Arc<AtomicUsize>,
        cancelled: Arc<AtomicBool>,
    ) -> Vec<(i32, usize)> {
        self.scan_list_with_limit(list, config, None, &progress, &cancelled)
    }
}

fn timestamp_setting(value: Option<&str>) -> Result<bool, &'static str> {
    match value {
        None | Some("1") => Ok(true),
        Some("0") => Ok(false),
        Some(_) => Err("ACS_GPU_TIMESTAMPS 必须为 0 或 1"),
    }
}

#[cfg(test)]
mod diagnostic_options_tests {
    use super::*;
    use crate::core::map_maker::ScoringStage;

    #[test]
    fn timestamp_option_is_strict_and_defaults_on() {
        assert_eq!(timestamp_setting(None), Ok(true));
        assert_eq!(timestamp_setting(Some("1")), Ok(true));
        assert_eq!(timestamp_setting(Some("0")), Ok(false));
        for value in ["", "true", "false", " 0", "2"] {
            assert!(timestamp_setting(Some(value)).is_err());
        }
    }

    #[test]
    fn invalid_gpu_option_does_not_disable_cpu_oracle() {
        let mut scanner = EngineScanner::new(ComputeMode::CpuRayon);
        // No environment mutation and no GPU initialization in this test.
        scanner.gpu_timestamp_config_error = Some("invalid test option".to_owned());
        let config = ScanConfig {
            map_size: 1,
            threshold: 0,
            scoring_stage: ScoringStage::RawLing6,
        };
        let progress = Arc::new(AtomicUsize::new(0));
        let output = scanner.scan_cancellable(
            1,
            1,
            &config,
            Arc::clone(&progress),
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(output.len(), 1);
        assert_eq!(progress.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(scanner.backend_status(), BackendStatus::Cpu);
        assert_eq!(scanner.actual_gpu_timestamps(), None);
        scanner.mode = ComputeMode::HeterogeneousPipeline;
        assert!(!scanner.valid_config(&config));
        assert!(matches!(
            scanner.backend_status(),
            BackendStatus::InvalidConfig(_)
        ));
    }
}
