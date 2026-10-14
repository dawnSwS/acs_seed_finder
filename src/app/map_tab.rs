use crate::app::task::BackgroundTask;
use crate::core::map_maker::{MapMaker, ScoringStage};
use crate::core::scanner_api::{ComputeMode, ScanConfig};
use crate::core::seed_input::{inclusive_seed_count, parse_seed_list};
use crate::core::utils::{find_chinese_collision, string_hash};
use crate::scanner::{BackendStatus, EngineScanner, GpuKernelCandidate};
use eframe::egui;
use serde_json::json;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

#[path = "benchmark_report.rs"]
mod benchmark_report;
use benchmark_report::BenchmarkReport;

type MapResults = Vec<(i32, usize, String)>;

const BENCHMARK_SEED_START: i32 = 10_000;
const BENCHMARK_SEED_COUNT: i32 = 2_000;
const BENCHMARK_SEED_END: i32 = BENCHMARK_SEED_START + BENCHMARK_SEED_COUNT - 1;
const BENCHMARK_WORKLOAD_COUNT: usize = 2;

fn benchmark_total_seeds(candidate_count: usize) -> usize {
    BENCHMARK_SEED_COUNT as usize * BENCHMARK_WORKLOAD_COUNT * candidate_count
}

fn scoring_stage_label(stage: ScoringStage) -> &'static str {
    match stage {
        ScoringStage::RawLing6 => "第六轮原始灵土",
        ScoringStage::Postprocessed => "环境处理后灵土",
    }
}

pub struct ScanOutcome {
    results: MapResults,
    message: String,
    description: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BenchmarkWorkload {
    AllCounts,
    Threshold600,
}

impl BenchmarkWorkload {
    fn label(self) -> &'static str {
        match self {
            Self::AllCounts => "全量校验（阈值 0）",
            Self::Threshold600 => "实际扫描（阈值 600）",
        }
    }

    fn threshold(self) -> usize {
        match self {
            Self::AllCounts => 0,
            Self::Threshold600 => 600,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BenchmarkCandidate {
    Cpu,
    V3,
    Cooperative,
    CooperativeNoTimestamps,
    CooperativeWords,
}

impl BenchmarkCandidate {
    fn id(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::V3 => GpuKernelCandidate::V3.name(),
            Self::Cooperative => GpuKernelCandidate::Cooperative.name(),
            Self::CooperativeNoTimestamps => "cooperative-no-timestamps",
            Self::CooperativeWords => GpuKernelCandidate::CooperativeWords.name(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::V3 => "GPU v3",
            Self::Cooperative => "GPU 协作 / 请求时间戳（实验）",
            Self::CooperativeNoTimestamps => "GPU 协作 / 无时间戳（实验）",
            Self::CooperativeWords => "GPU 协作整字 / 无时间戳（实验）",
        }
    }

    fn mode(self) -> ComputeMode {
        match self {
            Self::Cpu => ComputeMode::CpuRayon,
            Self::V3
            | Self::Cooperative
            | Self::CooperativeNoTimestamps
            | Self::CooperativeWords => ComputeMode::HeterogeneousPipeline,
        }
    }

    fn requested_kernel(self) -> Option<&'static str> {
        match self {
            Self::Cpu => None,
            Self::V3 => Some(GpuKernelCandidate::V3.name()),
            Self::Cooperative | Self::CooperativeNoTimestamps => {
                Some(GpuKernelCandidate::Cooperative.name())
            }
            Self::CooperativeWords => Some(GpuKernelCandidate::CooperativeWords.name()),
        }
    }

    fn experimental(self) -> bool {
        matches!(
            self,
            Self::Cooperative | Self::CooperativeNoTimestamps | Self::CooperativeWords
        )
    }

    fn scanner(self) -> EngineScanner {
        match self {
            Self::Cpu => EngineScanner::new(ComputeMode::CpuRayon),
            Self::V3 => EngineScanner::new_gpu_candidate(GpuKernelCandidate::V3),
            Self::Cooperative => EngineScanner::new_gpu_candidate(GpuKernelCandidate::Cooperative),
            Self::CooperativeNoTimestamps => {
                EngineScanner::new_gpu_candidate(GpuKernelCandidate::Cooperative)
                    .with_gpu_timestamps(false)
            }
            Self::CooperativeWords => {
                EngineScanner::new_gpu_candidate(GpuKernelCandidate::CooperativeWords)
                    .with_gpu_timestamps(false)
            }
        }
    }
}

fn benchmark_candidates(
    include_cooperative: bool,
    include_cooperative_words: bool,
) -> Vec<BenchmarkCandidate> {
    let mut candidates = vec![BenchmarkCandidate::Cpu, BenchmarkCandidate::V3];
    if include_cooperative {
        candidates.push(BenchmarkCandidate::Cooperative);
        candidates.push(BenchmarkCandidate::CooperativeNoTimestamps);
    }
    if include_cooperative_words {
        candidates.push(BenchmarkCandidate::CooperativeWords);
    }
    candidates
}

pub struct BenchmarkResult {
    workload: BenchmarkWorkload,
    candidate: BenchmarkCandidate,
    actual_kernel: Option<&'static str>,
    requested_gpu_timestamps: Option<bool>,
    actual_gpu_timestamps: Option<bool>,
    timestamp_expectation_pass: Option<bool>,
    gpu_performance_eligible: bool,
    seconds: f64,
    backend: BackendStatus,
    result_count: usize,
    comparison: &'static str,
    validation_passed: bool,
}

pub struct BenchmarkOutcome {
    results: Vec<BenchmarkResult>,
    report_directory: PathBuf,
    error: Option<String>,
}

pub struct MapTabState {
    pub map_size: i32,
    pub seed_start: i32,
    pub seed_end: i32,
    pub threshold: usize,
    pub scoring_stage: ScoringStage,
    pub limit_top_50: bool,
    pub string_seed: String,
    pub compute_mode: ComputeMode,
    pub status_msg: String,
    pub results: MapResults,
    pub result_description: String,
    pub benchmark_results: Vec<BenchmarkResult>,
    pub benchmark_include_cooperative: bool,
    pub benchmark_include_cooperative_words: bool,
    pub benchmark_report_directory: Option<PathBuf>,
    pub debug_report_directory: Option<PathBuf>,
    pub search_task: BackgroundTask<ScanOutcome>,
    pub bench_task: BackgroundTask<BenchmarkOutcome>,
}

impl Default for MapTabState {
    fn default() -> Self {
        Self {
            map_size: 192,
            seed_start: 0,
            seed_end: 999,
            threshold: 600,
            scoring_stage: ScoringStage::Postprocessed,
            limit_top_50: true,
            string_seed: String::new(),
            compute_mode: ComputeMode::CpuRayon,
            status_msg: String::new(),
            results: Vec::new(),
            result_description: String::new(),
            benchmark_results: Vec::new(),
            benchmark_include_cooperative: cfg!(feature = "gpu"),
            benchmark_include_cooperative_words: cfg!(feature = "gpu"),
            benchmark_report_directory: None,
            debug_report_directory: None,
            search_task: BackgroundTask::default(),
            bench_task: BackgroundTask::default(),
        }
    }
}

fn decorate_results(results: Vec<(i32, usize)>) -> MapResults {
    results
        .into_iter()
        .map(|(seed, count)| {
            (
                seed,
                count,
                find_chinese_collision(seed).unwrap_or_else(|| "无解".into()),
            )
        })
        .collect()
}

fn benchmark(
    progress: &Arc<AtomicUsize>,
    total: &AtomicUsize,
    cancel: &Arc<AtomicBool>,
    include_cooperative: bool,
    include_cooperative_words: bool,
) -> Result<BenchmarkOutcome, String> {
    let candidates = benchmark_candidates(include_cooperative, include_cooperative_words);
    let mut report = BenchmarkReport::new(json!({
        "map_size":192, "stage":"Postprocessed", "thresholds":[0,600],
        "seed_start":BENCHMARK_SEED_START, "seed_end":BENCHMARK_SEED_END,
        "seeds_per_run":BENCHMARK_SEED_COUNT, "timed_runs":BENCHMARK_WORKLOAD_COUNT * candidates.len(),
        "total_timed_seeds":benchmark_total_seeds(candidates.len()),
        "candidates":candidates.iter().map(|candidate|candidate.id()).collect::<Vec<_>>(),
        "include_cooperative_experiment":include_cooperative,
        "include_cooperative_words_experiment":include_cooperative_words,
        "kernel_selection":"GPU candidates explicitly selected per scanner; ACS_GPU_KERNEL does not select benchmark candidates",
        "timestamp_ab":"cooperative and cooperative-no-timestamps request the same kernel; per-row actual_gpu_timestamps records feature availability and query creation",
        "top_k":null, "warmup_seed":BENCHMARK_SEED_START-1,
        "warmup_seeds_per_backend":1, "repeats_per_workload_and_backend":1
    }))?;
    let mut timings = Vec::new();
    let measurement = run_benchmark(
        progress,
        total,
        cancel,
        &candidates,
        &mut timings,
        &mut report,
    );
    let measurement_error = measurement.err();
    let export_error = report
        .finish(measurement_error.as_deref(), cancel.load(Ordering::Relaxed))
        .err();
    let errors = measurement_error
        .into_iter()
        .chain(export_error)
        .collect::<Vec<_>>();
    Ok(BenchmarkOutcome {
        results: timings,
        report_directory: report.directory().to_owned(),
        error: (!errors.is_empty()).then(|| errors.join("；")),
    })
}

fn run_benchmark(
    progress: &Arc<AtomicUsize>,
    total: &AtomicUsize,
    cancel: &Arc<AtomicBool>,
    candidates: &[BenchmarkCandidate],
    timings: &mut Vec<BenchmarkResult>,
    report: &mut BenchmarkReport,
) -> Result<(), String> {
    let mut full_reference: Option<Vec<(i32, usize)>> = None;
    let mut validation_errors = Vec::new();
    let mut config = ScanConfig {
        map_size: 192,
        // Compare every count, including seeds below the ordinary threshold.
        threshold: 0,
        scoring_stage: ScoringStage::Postprocessed,
    };
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    // Shared coefficient/geometry tables, the Rayon pool, and GPU discovery
    // are setup costs. Warm every candidate before either timed workload.
    report.set_run_id("setup-outline-tables");
    let setup_started = std::time::Instant::now();
    MapMaker::prepare_indexed_outline_tables();
    report.record_stage(json!({"stage":"outline_tables", "seconds":setup_started.elapsed().as_secs_f64(), "included_in_scan_seconds":false}));
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    for &candidate in candidates {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let run_id = format!("warmup-{}", candidate.id());
        report.set_run_id(&run_id);
        let scanner = candidate.scanner();
        let warmup_started = std::time::Instant::now();
        let warmup_results = scanner.scan_cancellable(
            BENCHMARK_SEED_START - 1,
            BENCHMARK_SEED_START - 1,
            &config,
            Arc::new(AtomicUsize::new(0)),
            Arc::clone(cancel),
        );
        report.record_stage(json!({
            "stage":"warmup", "run_id":run_id, "requested_mode":benchmark_mode_id(candidate.mode()),
            "candidate":candidate.id(), "requested_kernel":candidate.requested_kernel(),
            "actual_kernel":scanner.actual_gpu_kernel(),
            "requested_gpu_timestamps":(candidate != BenchmarkCandidate::Cpu).then(|| scanner.requested_gpu_timestamps()),
            "actual_gpu_timestamps":scanner.actual_gpu_timestamps(),
            "experimental_candidate":candidate.experimental(),
            "seconds":warmup_started.elapsed().as_secs_f64(), "included_in_scan_seconds":false,
            "input_seeds":1, "seed_start":BENCHMARK_SEED_START-1,
            "returned_items":warmup_results.len(), "backend_status":format!("{:?}", scanner.backend_status()),
            "cancelled":cancel.load(Ordering::Relaxed)
        }));
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    total.store(benchmark_total_seeds(candidates.len()), Ordering::Relaxed);
    for workload in [
        BenchmarkWorkload::AllCounts,
        BenchmarkWorkload::Threshold600,
    ] {
        config.threshold = workload.threshold();
        // This filtered oracle is outside scan timing. Each timed scanner still
        // generates every benchmark seed and performs its own filtering and sorting.
        let filtered_reference = full_reference.as_ref().map(|all| {
            all.iter()
                .copied()
                .filter(|(_, count)| *count >= config.threshold)
                .collect::<Vec<_>>()
        });
        for &candidate in candidates {
            if cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            let run_id = format!("threshold{}-{}", config.threshold, candidate.id());
            report.set_run_id(&run_id);
            let scanner = candidate.scanner();
            let completed_before = progress.load(Ordering::Relaxed);
            let start = std::time::Instant::now();
            let results = scanner.scan_cancellable(
                BENCHMARK_SEED_START,
                BENCHMARK_SEED_END,
                &config,
                Arc::clone(progress),
                Arc::clone(cancel),
            );
            let seconds = start.elapsed().as_secs_f64();
            let backend = scanner.backend_status();
            let actual_kernel = scanner.actual_gpu_kernel();
            let requested_gpu_timestamps =
                (candidate != BenchmarkCandidate::Cpu).then(|| scanner.requested_gpu_timestamps());
            let actual_gpu_timestamps = scanner.actual_gpu_timestamps();
            let completed = progress
                .load(Ordering::Relaxed)
                .saturating_sub(completed_before);
            let expected = match workload {
                BenchmarkWorkload::AllCounts => full_reference.as_ref(),
                BenchmarkWorkload::Threshold600 => filtered_reference.as_ref(),
            };
            let cancelled = cancel.load(Ordering::Relaxed);
            let exact_equal = expected.map(|expected| expected == &results);
            let complete = completed == BENCHMARK_SEED_COUNT as usize && !cancelled;
            let hardware_gpu_backend = match &backend {
                #[cfg(feature = "gpu")]
                BackendStatus::Gpu => true,
                _ => false,
            };
            // A cancelled/partial GPU row is not a complete hardware execution.
            // Count and kernel correctness are reported separately below.
            let pure_hardware_gpu = hardware_gpu_backend && complete;
            let kernel_expectation_pass =
                hardware_gpu_backend.then(|| actual_kernel == candidate.requested_kernel());
            let timestamp_expectation_pass =
                hardware_gpu_backend.then(|| requested_gpu_timestamps == actual_gpu_timestamps);
            let validation_passed =
                complete && exact_equal != Some(false) && kernel_expectation_pass != Some(false);
            let gpu_performance_eligible =
                pure_hardware_gpu && validation_passed && timestamp_expectation_pass == Some(true);
            report.record_run(&run_id, json!({
                "requested_mode":benchmark_mode_id(candidate.mode()), "workload":workload.label(),
                "candidate":candidate.id(), "requested_kernel":candidate.requested_kernel(),
                "actual_kernel":actual_kernel,
                "requested_gpu_timestamps":requested_gpu_timestamps,
                "actual_gpu_timestamps":actual_gpu_timestamps,
                "experimental_candidate":candidate.experimental(),
                "seconds":seconds, "map_size":config.map_size, "stage":"Postprocessed",
                "threshold":config.threshold, "seed_start":BENCHMARK_SEED_START,
                "seed_end":BENCHMARK_SEED_END, "input_seeds":BENCHMARK_SEED_COUNT,
                "completed_seeds":completed, "returned_items":results.len(),
                "returned_count_sum":results.iter().map(|(_,count)| *count).sum::<usize>(),
                "backend_status":format!("{backend:?}"), "backend_description":backend.to_string(),
                "pure_hardware_gpu":pure_hardware_gpu, "exact_equal":exact_equal,
                "kernel_expectation_pass":kernel_expectation_pass,
                "timestamp_expectation_pass":timestamp_expectation_pass,
                "gpu_performance_eligible":gpu_performance_eligible,
                "validation_passed":validation_passed,
                "reference":if expected.is_none(){"CPU full-count reference"}else{"CPU full-count reference, filtered to requested threshold"},
                "cancelled":cancelled, "complete":complete,
                "timing":"complete warmed scanner call; result export and equality checks outside interval"
            }), &results)?;
            if cancelled {
                return Ok(());
            }
            if !complete {
                validation_errors.push(format!(
                    "{} / {} 未完成全部种子：{backend}",
                    workload.label(),
                    candidate.label()
                ));
            }
            if exact_equal == Some(false) {
                validation_errors.push(format!(
                    "{} / {} 的计数、筛选或排序与 CPU 全量参考不一致：{backend}",
                    workload.label(),
                    candidate.label()
                ));
            }
            if kernel_expectation_pass == Some(false) {
                validation_errors.push(format!(
                    "{} / {} 实际内核不符：请求 {}，实际 {}；{backend}",
                    workload.label(),
                    candidate.label(),
                    candidate.requested_kernel().unwrap_or("无 GPU 内核"),
                    actual_kernel.unwrap_or("未记录 GPU 内核")
                ));
            }
            let comparison = if kernel_expectation_pass == Some(false) {
                "内核不符（验证失败）"
            } else if !complete {
                "未完成全部种子"
            } else if exact_equal == Some(false) {
                "不一致（验证失败）"
            } else if expected.is_none() {
                "CPU 全量参考"
            } else {
                benchmark_comparison(&backend)
            };
            timings.push(BenchmarkResult {
                workload,
                candidate,
                actual_kernel,
                requested_gpu_timestamps,
                actual_gpu_timestamps,
                timestamp_expectation_pass,
                gpu_performance_eligible,
                seconds,
                backend,
                result_count: results.len(),
                comparison,
                validation_passed,
            });
            if workload == BenchmarkWorkload::AllCounts && candidate == BenchmarkCandidate::Cpu {
                if !complete {
                    return Err(validation_errors.join("；"));
                }
                full_reference = Some(results);
            }
        }
    }
    if validation_errors.is_empty() {
        Ok(())
    } else {
        Err(validation_errors.join("；"))
    }
}

fn benchmark_mode_id(mode: ComputeMode) -> &'static str {
    match mode {
        ComputeMode::CpuRayon => "cpu",
        ComputeMode::HeterogeneousPipeline => "gpu",
    }
}

fn debug_status_message(status: &str) -> String {
    // Import validation can echo an invalid field from the user's text file.
    // Preserve the location/error while excluding that original input text.
    if let Some((prefix, field_and_error)) = status.split_once("行种子无效 (") {
        if let Some((_, error)) = field_and_error.rsplit_once("): ") {
            return format!("{prefix}行种子无效 ([原始字段已省略]): {error}");
        }
    }
    status.to_owned()
}

fn benchmark_comparison(backend: &BackendStatus) -> &'static str {
    match backend {
        BackendStatus::Cpu => "与 CPU 全量参考一致",
        #[cfg(feature = "gpu")]
        BackendStatus::Gpu => "一致（纯硬件 GPU）",
        #[cfg(feature = "gpu")]
        BackendStatus::SoftwareGpu {
            cpu_fallback_seeds: 0,
            ..
        } => "一致（软件设备，非硬件 GPU）",
        #[cfg(feature = "gpu")]
        BackendStatus::SoftwareGpu { .. } | BackendStatus::GpuWithCpuFallback { .. } => {
            "一致（含 CPU 回退，非纯 GPU 验证）"
        }
        BackendStatus::CpuFallback(_) => "一致（CPU 回退，未验证 GPU）",
        BackendStatus::InvalidConfig(_) => "配置无效",
    }
}

impl MapTabState {
    pub fn is_busy(&self) -> bool {
        self.search_task.is_running || self.bench_task.is_running
    }

    pub fn update(&mut self) {
        self.update_search();
        self.update_benchmark();
    }

    fn update_search(&mut self) {
        if let Some(result) = self.search_task.poll() {
            match result {
                Ok(outcome) => {
                    self.results = outcome.results;
                    self.result_description = outcome.description;
                    self.status_msg = outcome.message;
                }
                Err(error) => self.status_msg = format!("扫描失败: {error}"),
            }
        }
    }

    fn update_benchmark(&mut self) {
        if self.bench_task.is_running {
            self.status_msg = if self.bench_task.cancelled.load(Ordering::Relaxed) {
                "正在停止基准测试...".into()
            } else if self.bench_task.total_tasks.load(Ordering::Relaxed) == 0 {
                "正在准备基准测试：预热所选候选（不计入耗时）...".into()
            } else {
                format!(
                    "正在测试：全量校验与阈值 600 扫描，每个候选各运行一次，共 {} 个种子...",
                    self.bench_task.total_tasks.load(Ordering::Relaxed)
                )
            };
        }
        if let Some(result) = self.bench_task.poll() {
            match result {
                Ok(outcome) => {
                    self.benchmark_results = outcome.results;
                    self.benchmark_report_directory = Some(outcome.report_directory);
                    self.status_msg = if let Some(error) = outcome.error {
                        format!("基准测试或报告导出失败：{error}；报告目录见下方")
                    } else if self.bench_task.cancelled.load(Ordering::Relaxed) {
                        "基准测试已停止".into()
                    } else {
                        "基准测试完成；原始结果与 GPU 分阶段诊断已自动保存，报告目录见下方".into()
                    };
                }
                Err(error) => self.status_msg = format!("基准测试失败: {error}"),
            }
        }
    }

    fn validated_config(&mut self) -> Option<ScanConfig> {
        let config = ScanConfig {
            map_size: self.map_size,
            threshold: self.threshold,
            scoring_stage: self.scoring_stage,
        };
        match config.validate() {
            Ok(()) => Some(config),
            Err(error) => {
                self.status_msg = format!("参数无效: {error}");
                None
            }
        }
    }

    pub fn start_search(&mut self) {
        if self.is_busy() {
            return;
        }
        let Some(config) = self.validated_config() else {
            return;
        };
        let total = match inclusive_seed_count(self.seed_start, self.seed_end) {
            Ok(total) => total,
            Err(error) => {
                self.status_msg = error;
                return;
            }
        };
        let (start, end, top_50, mode) = (
            self.seed_start,
            self.seed_end,
            self.limit_top_50,
            self.compute_mode,
        );
        let result = self.search_task.start(total, move |progress, _, cancel| {
            let scanner = EngineScanner::new(mode);
            let results = if top_50 {
                scanner.scan_top_cancellable(start, end, &config, 50, progress, Arc::clone(&cancel))
            } else {
                scanner.scan_cancellable(start, end, &config, progress, Arc::clone(&cancel))
            };
            let message = format!(
                "{}；计算后端: {}",
                if cancel.load(Ordering::Relaxed) {
                    "已停止，显示已完成结果"
                } else {
                    "扫描完成"
                },
                scanner.backend_status()
            );
            Ok(ScanOutcome {
                results: decorate_results(results),
                message,
                description: format!(
                    "地图 {0}×{0}，阈值 {1}，{2}，种子 {start}..={end}",
                    config.map_size,
                    config.threshold,
                    scoring_stage_label(config.scoring_stage)
                ),
            })
        });
        self.status_msg = result.map_or_else(|error| error, |()| "正在扫描...".into());
    }

    pub fn run_bench(&mut self) {
        if self.is_busy() {
            return;
        }
        self.benchmark_results.clear();
        self.benchmark_report_directory = None;
        let include_cooperative = self.benchmark_include_cooperative && cfg!(feature = "gpu");
        let include_cooperative_words =
            self.benchmark_include_cooperative_words && cfg!(feature = "gpu");
        let result = self.bench_task.start(0, move |progress, total, cancel| {
            benchmark(
                &progress,
                &total,
                &cancel,
                include_cooperative,
                include_cooperative_words,
            )
        });
        self.status_msg = result.map_or_else(
            |error| error,
            |()| "正在准备基准测试：预热所选候选（不计入耗时）...".into(),
        );
    }

    pub fn import_seeds(&mut self) {
        if self.is_busy() {
            return;
        }
        let Some(config) = self.validated_config() else {
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("文本", &["txt", "csv"])
            .pick_file()
        else {
            return;
        };
        let top_50 = self.limit_top_50;
        let mode = self.compute_mode;
        let result = self.search_task.start(0, move |progress, total, cancel| {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("读取 {} 失败: {error}", path.display()))?;
            let seeds = parse_seed_list(&text)?;
            let seed_total = seeds.len();
            total.store(seed_total, Ordering::Relaxed);
            let scanner = EngineScanner::new(mode);
            let results = if top_50 {
                scanner.scan_list_top_cancellable(seeds, &config, 50, progress, Arc::clone(&cancel))
            } else {
                scanner.scan_list_cancellable(seeds, &config, progress, Arc::clone(&cancel))
            };
            Ok(ScanOutcome {
                results: decorate_results(results),
                description: format!(
                    "地图 {0}×{0}，阈值 {1}，{3}，列表 {seed_total} 个种子，文件 {2}",
                    config.map_size,
                    config.threshold,
                    path.display(),
                    scoring_stage_label(config.scoring_stage)
                ),
                message: format!(
                    "{}；计算后端: {}",
                    if cancel.load(Ordering::Relaxed) {
                        "已停止"
                    } else {
                        "导入扫描完成"
                    },
                    scanner.backend_status()
                ),
            })
        });
        self.status_msg =
            result.map_or_else(|error| error, |()| "正在读取并扫描种子文件...".into());
    }

    pub fn render(&mut self, ui: &mut egui::Ui) {
        self.render_config(ui);
        self.render_controls(ui);
        self.render_benchmark(ui);
        self.render_results(ui);
    }

    fn render_config(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("cfg")
            .num_columns(2)
            .spacing([40.0, 15.0])
            .show(ui, |ui| {
                ui.label("地图尺寸:");
                ui.horizontal(|u| {
                    [96, 128, 192]
                        .into_iter()
                        .zip(["小型", "中型", "大型"])
                        .for_each(|(v, t)| {
                            u.radio_value(&mut self.map_size, v, t);
                        });
                });
                ui.end_row();
                ui.label("计算模式:");
                ui.horizontal(|u| {
                    for (mode, label) in [
                        (ComputeMode::CpuRayon, "CPU"),
                        (ComputeMode::HeterogeneousPipeline, "⚡ GPU 生成（实验）"),
                    ] {
                        u.radio_value(&mut self.compute_mode, mode, label);
                    }
                });
                ui.end_row();
                ui.label("统计阶段:");
                ui.horizontal(|u| {
                    for stage in [ScoringStage::RawLing6, ScoringStage::Postprocessed] {
                        u.radio_value(&mut self.scoring_stage, stage, scoring_stage_label(stage));
                    }
                });
                ui.end_row();
                ui.label("输入文本:");
                ui.horizontal(|u| {
                    u.text_edit_singleline(&mut self.string_seed);
                    if u.button("转为数字种子").clicked() && !self.string_seed.is_empty() {
                        self.seed_start = string_hash(&self.string_seed);
                        self.seed_end = self.seed_start;
                    }
                });
                ui.end_row();
                ui.label("种子区间:");
                ui.horizontal(|u| {
                    u.add(egui::DragValue::new(&mut self.seed_start).prefix("起: "));
                    u.label("至");
                    u.add(egui::DragValue::new(&mut self.seed_end).prefix("止: "));
                });
                ui.end_row();
                ui.label("灵土阈值:");
                ui.horizontal(|u| {
                    u.add(
                        egui::DragValue::new(&mut self.threshold)
                            .speed(10)
                            .suffix(" 格"),
                    );
                    u.add_space(20.0);
                    u.checkbox(&mut self.limit_top_50, "仅 Top 50");
                });
                ui.end_row();
            });
        ui.add_space(20.0);
    }

    fn render_controls(&mut self, ui: &mut egui::Ui) {
        if self.search_task.is_running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("扫描中...");
                if ui.button("停止扫描").clicked() {
                    self.search_task.cancel();
                }
            });
            ui.add(
                egui::ProgressBar::new(self.search_task.fraction())
                    .show_percentage()
                    .text(format!(
                        "{}/{}",
                        self.search_task.progress.load(Ordering::Relaxed),
                        self.search_task.total_tasks.load(Ordering::Relaxed)
                    )),
            );
        } else {
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!self.bench_task.is_running, |ui| {
                    if ui.button("开始扫描").clicked() {
                        self.start_search();
                    }
                    ui.add_space(10.0);
                    if ui.button("导入种子列表").clicked() {
                        self.import_seeds();
                    }
                })
            });
            if !self.status_msg.is_empty() {
                ui.label(egui::RichText::new(&self.status_msg).color(egui::Color32::YELLOW));
            }
        }
        ui.separator();
    }

    fn render_benchmark(&mut self, ui: &mut egui::Ui) {
        self.render_benchmark_controls(ui);
        ui.small(format!("固定输入：192×192，环境处理后灵土，种子 {BENCHMARK_SEED_START}–{BENCHMARK_SEED_END}（每路 {BENCHMARK_SEED_COUNT} 个）；全量校验（阈值 0）与实际扫描（阈值 600）分别运行 CPU、v3 及勾选的协作候选。"));
        ui.small("预热后各计时一轮，包含全部种子的计算、结果筛选和排序；对照检查不计时。正式冷启动与重复性能验证使用独立验证入口。");
        ui.small("协作时间戳开/关对照使用同一内核；关闭时间戳只停止设备计时查询，不代表 CPU 回退。实际状态以结果列和日志为准。");
        ui.small("基准显式选择每个 GPU 内核，无需环境变量。自动导出原始秒数、有序计数与分阶段诊断；本次报告写盘在扫描计时之外。");
        if let Some(directory) = self.benchmark_report_directory.clone() {
            ui.label(format!("报告目录：{}", directory.display()));
            ui.horizontal(|ui| {
                if ui.button("打开报告目录").clicked() {
                    if let Err(error) = benchmark_report::open_directory(&directory) {
                        self.status_msg = error;
                    }
                }
                if ui.button("复制报告路径").clicked() {
                    ui.output_mut(|output| output.copied_text = directory.display().to_string());
                }
            });
        }
        if let Some(directory) = self.debug_report_directory.clone() {
            ui.label(format!("Debug 日志目录：{}", directory.display()));
            ui.horizontal(|ui| {
                if ui.button("打开 Debug 日志目录").clicked() {
                    if let Err(error) = benchmark_report::open_directory(&directory) {
                        self.status_msg = error;
                    }
                }
                if ui.button("复制 Debug 日志路径").clicked() {
                    ui.output_mut(|output| output.copied_text = directory.display().to_string());
                }
            });
        }
        if !self.benchmark_results.is_empty() {
            self.render_benchmark_table(ui);
        }
        ui.separator();
    }

    fn render_benchmark_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("⚡ 基准测试");
            if self.bench_task.is_running {
                ui.spinner();
                if ui.button("停止测试").clicked() {
                    self.bench_task.cancel();
                }
            } else if ui
                .button(format!("运行校验与 {BENCHMARK_SEED_COUNT} 种子扫描"))
                .clicked()
                && !self.search_task.is_running
            {
                self.run_bench();
            }
            if ui.add_enabled(!self.is_busy(), egui::Button::new("导出 Debug 日志"))
                .on_hover_text("导出最近的 GPU 诊断和设备/版本信息；普通扫描结束或失败后也可使用，不会启动 GPU 计算。只需发回 debug-report.json，无需打包目录")
                .clicked()
            {
                self.debug_report_directory = None;
                let benchmark_results = self.benchmark_results.iter().map(|result| json!({
                    "workload":result.workload.label(),
                    "threshold":result.workload.threshold(),
                    "mode":benchmark_mode_id(result.candidate.mode()),
                    "candidate":result.candidate.id(),
                    "requested_kernel":result.candidate.requested_kernel(),
                    "actual_kernel":result.actual_kernel,
                    "requested_gpu_timestamps":result.requested_gpu_timestamps,
                    "actual_gpu_timestamps":result.actual_gpu_timestamps,
                    "timestamp_expectation_pass":result.timestamp_expectation_pass,
                    "gpu_performance_eligible":result.gpu_performance_eligible,
                    "experimental_candidate":result.candidate.experimental(),
                    "seconds":result.seconds,
                    "backend":format!("{:?}", result.backend),
                    "backend_description":result.backend.to_string(),
                    "result_count":result.result_count,
                    "comparison":result.comparison,
                    "validation_passed":result.validation_passed
                })).collect::<Vec<_>>();
                let context = json!({
                    "scope":"Current GUI controls, not necessarily the inputs of the last scan. Original text/seed lists are excluded.",
                    "map_size":self.map_size, "threshold":self.threshold,
                    "stage":format!("{:?}", self.scoring_stage),
                    "requested_mode":benchmark_mode_id(self.compute_mode),
                    "top_k":if self.limit_top_50 {Some(50)} else {None},
                    "configured_range_seed_count":inclusive_seed_count(self.seed_start, self.seed_end).ok(),
                    "displayed_result_count":self.results.len(),
                    "last_status_message":debug_status_message(&self.status_msg),
                    "benchmark_report_directory":self.benchmark_report_directory,
                    "include_cooperative_experiment":self.benchmark_include_cooperative,
                    "include_cooperative_words_experiment":self.benchmark_include_cooperative_words,
                    "benchmark_results":benchmark_results
                });
                match benchmark_report::export_debug_log(context) {
                    Ok(directory) => {
                        self.status_msg = format!("Debug 日志已导出：{}；只需发回 debug-report.json", directory.display());
                        self.debug_report_directory = Some(directory);
                    }
                    Err(error) => self.status_msg = format!("Debug 日志导出失败：{error}"),
                }
            }
        });
        ui.add_enabled(
            cfg!(feature = "gpu") && !self.is_busy(),
            egui::Checkbox::new(
                &mut self.benchmark_include_cooperative,
                "包含协作实验候选及时间戳开/关对照（未通过硬件验收）",
            ),
        ).on_hover_text("增加协作请求时间戳和协作无时间戳两路，同一着色器用于测量查询开销；可与下方整字候选独立选择。");
        ui.add_enabled(
            cfg!(feature = "gpu") && !self.is_busy(),
            egui::Checkbox::new(
                &mut self.benchmark_include_cooperative_words,
                "包含协作整字实验候选（关闭时间戳，未通过硬件验收）",
            ),
        ).on_hover_text("使用整字事件处理候选，与协作无时间戳行直接比较；仍须逐种子通过 CPU 参考校验，不代表已有硬件性能结论。");
        let candidate_count = benchmark_candidates(
            self.benchmark_include_cooperative && cfg!(feature = "gpu"),
            self.benchmark_include_cooperative_words && cfg!(feature = "gpu"),
        )
        .len();
        ui.small(format!(
            "当前设置：{} 路候选 × {} 种工作量 = {} 项，每项 {BENCHMARK_SEED_COUNT} 个种子，总进度 {}。",
            candidate_count, BENCHMARK_WORKLOAD_COUNT,
            candidate_count * BENCHMARK_WORKLOAD_COUNT, benchmark_total_seeds(candidate_count)
        ));
    }

    fn render_benchmark_table(&self, ui: &mut egui::Ui) {
        egui::ScrollArea::horizontal()
            .id_source("bench_scroll")
            .show(ui, |ui| {
                egui::Grid::new("bench")
                    .striped(true)
                    .spacing([16.0, 10.0])
                    .show(ui, |ui| {
                        for heading in [
                            "工作量",
                            "请求候选",
                            "实际内核",
                            "时间戳 请求/实际",
                            "耗时(s)",
                            "吞吐量(S/s)",
                            "倍率",
                            "返回项数",
                            "实际后端",
                            "对照结果",
                        ] {
                            ui.strong(heading);
                        }
                        ui.end_row();
                        for result in &self.benchmark_results {
                            let baseline = self
                                .benchmark_results
                                .iter()
                                .find(|reference| {
                                    reference.workload == result.workload
                                        && reference.candidate == BenchmarkCandidate::Cpu
                                        && reference.validation_passed
                                })
                                .map(|reference| reference.seconds);
                            render_benchmark_row(ui, result, baseline);
                        }
                    });
            });
        for result in &self.benchmark_results {
            if !matches!(result.backend, BackendStatus::Cpu) {
                ui.small(format!(
                    "{} / {}：{}",
                    result.workload.label(),
                    result.candidate.label(),
                    result.backend
                ));
            }
        }
        ui.small("倍率只在同工作量、结果一致且时间戳状态符合请求的 CPU 与纯硬件 GPU 间比较；时间戳不符仍保留计数对照。软件设备、CPU 回退及混合结果不作为 GPU 加速成绩。协作候选仍属实验，单轮结果不代表硬件验收。");
    }

    fn render_results(&mut self, ui: &mut egui::Ui) {
        if !self.result_description.is_empty() {
            ui.label(&self.result_description);
        }
        self.render_results_header(ui);
        ui.add_space(10.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| self.render_results_table(ui));
    }

    fn render_results_header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(format!("扫描结果 (共 {} 个)", self.results.len()));
            if !self.results.is_empty() {
                ui.add_space(20.0);
                if ui.button("复制全部文本").clicked() {
                    ui.output_mut(|o| {
                        o.copied_text = self
                            .results
                            .iter()
                            .map(|(_, _, t)| t.clone())
                            .collect::<Vec<_>>()
                            .join("\n");
                    });
                }
                ui.add_space(10.0);
                if ui.button("复制全部数字").clicked() {
                    ui.output_mut(|o| {
                        o.copied_text = self
                            .results
                            .iter()
                            .map(|(s, _, _)| s.to_string())
                            .collect::<Vec<_>>()
                            .join("\n");
                    });
                }
            }
        });
    }

    fn render_results_table(&self, ui: &mut egui::Ui) {
        if self.results.is_empty() {
            return;
        }
        egui::Grid::new("res")
            .num_columns(4)
            .striped(true)
            .spacing([30.0, 10.0])
            .show(ui, |ui| {
                for heading in ["排名", "文本种子", "灵土数量", "数字种子"] {
                    ui.strong(heading);
                }
                ui.end_row();
                for (index, (seed, count, text)) in self.results.iter().enumerate() {
                    render_result_row(ui, index, *seed, *count, text);
                }
            });
    }
}

fn render_benchmark_row(ui: &mut egui::Ui, result: &BenchmarkResult, baseline: Option<f64>) {
    let time = result.seconds;
    let seconds = time.max(0.0001);
    let speedup = baseline.unwrap_or_default() / seconds;
    ui.label(result.workload.label());
    ui.label(result.candidate.label());
    ui.label(
        result
            .actual_kernel
            .unwrap_or(if result.candidate == BenchmarkCandidate::Cpu {
                "不适用"
            } else {
                "未调度 GPU"
            }),
    );
    ui.label(if result.candidate == BenchmarkCandidate::Cpu {
        "不适用".to_owned()
    } else {
        format!(
            "{} / {}",
            if result.requested_gpu_timestamps == Some(true) {
                "开"
            } else {
                "关"
            },
            match result.actual_gpu_timestamps {
                Some(true) => "开",
                Some(false) => "关",
                None => "未创建查询缓冲",
            }
        )
    })
    .on_hover_text("请求开启但实际关闭可表示设备不支持时间戳；这与 CPU 回退状态分开记录。");
    ui.label(format!("{time:.2} s"));
    ui.label(if result.validation_passed {
        format!("{:.0}", f64::from(BENCHMARK_SEED_COUNT) / seconds)
    } else {
        "—".into()
    });
    let ratio_allowed = result.validation_passed
        && baseline.is_some()
        && match &result.backend {
            BackendStatus::Cpu => true,
            #[cfg(feature = "gpu")]
            BackendStatus::Gpu => result.gpu_performance_eligible,
            _ => false,
        };
    let mut speedup_text = egui::RichText::new(if ratio_allowed {
        format!("{speedup:.2}x")
    } else {
        "—".into()
    });
    if ratio_allowed && speedup > 1.5 {
        speedup_text = speedup_text.color(egui::Color32::GREEN);
    }
    ui.label(speedup_text);
    ui.label(result.result_count.to_string());
    ui.label(match &result.backend {
        BackendStatus::Cpu => "CPU 并行扫描",
        #[cfg(feature = "gpu")]
        BackendStatus::Gpu => "GPU 生成与计数",
        #[cfg(feature = "gpu")]
        BackendStatus::SoftwareGpu {
            cpu_fallback_seeds: 0,
            ..
        } => "软件 GPU",
        #[cfg(feature = "gpu")]
        BackendStatus::SoftwareGpu { .. } => "软件 GPU + CPU 回退",
        #[cfg(feature = "gpu")]
        BackendStatus::GpuWithCpuFallback { .. } => "GPU + 部分 CPU 回退",
        BackendStatus::CpuFallback(_) => "CPU（回退）",
        BackendStatus::InvalidConfig(_) => "配置无效",
    })
    .on_hover_text(result.backend.to_string());
    ui.label(result.comparison);
    ui.end_row();
}

fn render_result_row(ui: &mut egui::Ui, index: usize, seed: i32, count: usize, text: &str) {
    ui.label(format!("#{}", index + 1));
    if ui.button(text).clicked() {
        ui.output_mut(|output| output.copied_text = text.into());
    }
    ui.label(format!("✨ {count}"));
    if ui.button(seed.to_string()).clicked() {
        ui.output_mut(|output| output.copied_text = seed.to_string());
    }
    ui.end_row();
}
