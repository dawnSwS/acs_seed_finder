//! GPU-resident exact generation. The CPU uploads a shared immutable RNG
//! coefficient table once and a four-word descriptor per seed. Maps stay on
//! the device; failed seeds are explicitly reported and recomputed once.
use super::{BackendStatus, ScanConfig};
use crate::core::{
    indexed_rng::{self, Seed},
    map_maker::{GenerationKernel, MapMaker, ScoringStage},
};
use crate::scanner::{cpu, GpuKernelCandidate as Kernel};
use serde_json::json;
use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc, Mutex, MutexGuard, OnceLock, TryLockError,
};
use std::time::{Duration, Instant};

#[path = "gpu_timing.rs"]
mod timing;

const DEFAULT_BATCH_SIZE: usize = 1024;
const MAX_BATCH_SIZE: usize = 2048;
const MAX_GPU_SIDE: i32 = 192;
const INPUT_WORDS: usize = 4;
const DIRS_CAPACITY: u32 = 4096;
const MAX_DRAWS: u32 = 2_000_000;
const SELF_TEST_SEEDS: [i32; 6] = [0, 1, 10_000, 161_803_399, i32::MIN, i32::MAX];

impl Kernel {
    fn index(self) -> usize {
        match self {
            Self::V3 => 0,
            Self::Cooperative => 1,
            Self::CooperativeWords => 2,
        }
    }
    fn selected() -> Result<Self, String> {
        match std::env::var("ACS_GPU_KERNEL").as_deref() {
            Ok("v3" | "") | Err(std::env::VarError::NotPresent) => Ok(Self::V3),
            Ok("cooperative") => Ok(Self::Cooperative),
            Ok("cooperative_words") => Ok(Self::CooperativeWords),
            _ => Err(
                "ACS_GPU_KERNEL 必须是 v3、cooperative 或 cooperative_words；更改环境后须重启程序"
                    .to_owned(),
            ),
        }
    }
    fn version(self) -> &'static str {
        match self {
            Self::V3 => "indexed-bitplanes-targetwise-v3",
            Self::Cooperative => "cooperative-packed-lds-v4-experimental",
            Self::CooperativeWords => "cooperative-streamed-words-v6-experimental",
        }
    }
    fn sources(self) -> (&'static str, &'static str) {
        match self {
            Self::V3 => (include_str!("gpu_rng.wgsl"), include_str!("generate.wgsl")),
            Self::Cooperative => (
                include_str!("gpu_rng_cooperative.wgsl"),
                include_str!("generate_cooperative.wgsl"),
            ),
            Self::CooperativeWords => (
                include_str!("gpu_rng_cooperative.wgsl"),
                include_str!("generate_cooperative_words.wgsl"),
            ),
        }
    }
    fn workgroups(self, maps: u32) -> u32 {
        match self {
            Self::V3 => maps.div_ceil(32),
            Self::Cooperative | Self::CooperativeWords => maps,
        }
    }
    fn cooperative(self) -> bool {
        matches!(self, Self::Cooperative | Self::CooperativeWords)
    }
}

struct GpuContext {
    kernel: Kernel,
    software: bool,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    coefficients: wgpu::Buffer,
    errors: Arc<Mutex<Option<String>>>,
    timestamp_state: Mutex<timing::TimestampState>,
    // Do not interleave concurrent scans' error scopes or validation operations.
    execution: Mutex<()>,
    validated: Mutex<HashSet<(i32, u32)>>,
}
struct ScanBuffers {
    config: wgpu::Buffer,
    inputs: wgpu::Buffer,
    output: wgpu::Buffer,
    readback: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    capacity: usize,
    timestamp_requested: bool,
    timestamps: Option<(wgpu::QuerySet, wgpu::Buffer)>,
}
#[derive(Clone, Copy, Debug)]
struct GpuOutput {
    count: usize,
    status: u32,
    draws: u32,
    queries: u32,
}
type ReadbackSignal = (Result<(), wgpu::BufferAsyncError>, Instant);
struct PreparedBatch {
    seeds: Vec<i32>,
    inputs: Vec<u32>,
    cpu_only: Vec<i32>,
}
static GPU_CONTEXTS: [OnceLock<Result<GpuContext, String>>; 3] =
    [OnceLock::new(), OnceLock::new(), OnceLock::new()];
static GPU_INITIALIZATIONS: [OnceLock<Result<(), String>>; 3] =
    [OnceLock::new(), OnceLock::new(), OnceLock::new()];
static NEXT_DISPATCH_ID: AtomicUsize = AtomicUsize::new(0);

struct GuiDiagnostics {
    run_id: String,
    events: Vec<serde_json::Value>,
}
static GUI_DIAGNOSTICS: Mutex<Option<GuiDiagnostics>> = Mutex::new(None);
static DEVICE_DIAGNOSTICS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
const RECENT_DIAGNOSTICS_LIMIT: usize = 4096;
static RECENT_DIAGNOSTICS: Mutex<VecDeque<serde_json::Value>> = Mutex::new(VecDeque::new());
static DIAGNOSTICS_DROPPED: AtomicUsize = AtomicUsize::new(0);

/// Snapshot the most recent events without initializing or submitting GPU work.
/// Identity is retained separately so it survives the bounded event window.
pub fn debug_diagnostics_snapshot() -> Vec<serde_json::Value> {
    let mut events = vec![json!({
        "event": "debug_snapshot", "schema_version": 1,
        "retained_event_limit": RECENT_DIAGNOSTICS_LIMIT,
        "dropped_events": DIAGNOSTICS_DROPPED.load(Ordering::Relaxed),
        "gpu_initialized": GPU_CONTEXTS.iter().any(|slot| slot.get().is_some()),
        "contexts": ([Kernel::V3, Kernel::Cooperative, Kernel::CooperativeWords].map(|kernel| json!({
            "kernel":kernel.name(), "initialization_finished":GPU_CONTEXTS[kernel.index()].get().is_some(),
            "initialization_error":GPU_CONTEXTS[kernel.index()].get().and_then(|result|result.as_ref().err())
        }))),
        "timing_note": "device_execution_ms is nested within submission_to_map_ms and overlaps host_wait_ms; queue.submit may block; do not add overlapping timers"
    })];
    events.extend(
        DEVICE_DIAGNOSTICS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .map(|mut value| {
                value["identity_snapshot"] = json!(true);
                value
            }),
    );
    events.extend(
        RECENT_DIAGNOSTICS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned(),
    );
    events
}

/// Capture GUI benchmark diagnostics in memory; file writing happens after timing.
pub fn begin_gui_diagnostics() -> Result<(), String> {
    let cached = DEVICE_DIAGNOSTICS
        .lock()
        .map_err(|_| "GPU 设备诊断缓存不可用".to_owned())?;
    let mut session = GUI_DIAGNOSTICS
        .lock()
        .map_err(|_| "GPU 诊断会话锁不可用".to_owned())?;
    if session.is_some() {
        return Err("已有 GPU 基准诊断会话正在运行".to_owned());
    }
    let events = cached
        .iter()
        .map(|event| {
            let mut value = event.clone();
            value["cached"] = json!(true);
            value["run_id"] = json!("benchmark-context");
            if value["event"] != "adapter" {
                value["event"] = json!(format!(
                    "cached_{}",
                    event["event"].as_str().unwrap_or("context")
                ));
            }
            value
        })
        .collect();
    *session = Some(GuiDiagnostics {
        run_id: "benchmark-context".to_owned(),
        events,
    });
    Ok(())
}

pub fn set_gui_diagnostic_run_id(run_id: &str) {
    if let Ok(mut session) = GUI_DIAGNOSTICS.lock() {
        if let Some(session) = session.as_mut() {
            run_id.clone_into(&mut session.run_id);
        }
    }
}

pub fn end_gui_diagnostics() -> Vec<serde_json::Value> {
    GUI_DIAGNOSTICS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .map_or_else(Vec::new, |session| session.events)
}

// JSONL is append-only; a failed export is visible on stderr, never a success flag.
fn diagnostic(mut value: serde_json::Value) {
    static WRITER: Mutex<()> = Mutex::new(());
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    value["event_sequence"] = json!(SEQUENCE.fetch_add(1, Ordering::Relaxed));
    value["elapsed_ms"] = json!(ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0);
    value["unix_time_ms"] = json!(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default());
    // Cache identity even when export was not enabled during the first scan.
    // Take/release this lock before the session lock, avoiding lock inversion.
    if matches!(
        value["event"].as_str(),
        Some("adapter" | "initialization" | "coefficient_setup")
    ) {
        DEVICE_DIAGNOSTICS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(value.clone());
    }
    {
        let mut session = GUI_DIAGNOSTICS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(session) = session.as_mut() {
            value["run_id"] = value
                .get("initialization_origin_run_id")
                .cloned()
                .unwrap_or_else(|| json!(session.run_id));
            session.events.push(value.clone());
        }
    }
    {
        let mut recent = RECENT_DIAGNOSTICS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if recent.len() == RECENT_DIAGNOSTICS_LIMIT {
            recent.pop_front();
            DIAGNOSTICS_DROPPED.fetch_add(1, Ordering::Relaxed);
        }
        recent.push_back(value.clone());
    }
    let Ok(path) = std::env::var("ACS_GPU_DIAGNOSTICS") else {
        return;
    };
    value["run_id"] = value
        .get("initialization_origin_run_id")
        .cloned()
        .unwrap_or_else(|| json!(std::env::var("ACS_VALIDATION_RUN_ID").ok()));
    let _guard = WRITER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| writeln!(file, "{value}"));
    if let Err(error) = result {
        eprintln!("GPU diagnostic export failed: {error}");
    }
}
fn shader_hash(kernel: Kernel) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    let (rng, generator) = kernel.sources();
    for b in rng.as_bytes().iter().chain(generator.as_bytes()) {
        hash = (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
fn shader_lf_hash(kernel: Kernel) -> String {
    let (rng, generator) = kernel.sources();
    let mut hash = 0xcbf29ce484222325u64;
    for b in rng
        .replace("\r\n", "\n")
        .bytes()
        .chain(generator.replace("\r\n", "\n").bytes())
    {
        hash = (hash ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
fn test_option(name: &str) -> Option<u32> {
    (std::env::var("ACS_GPU_TEST_MODE").as_deref() == Ok("1"))
        .then(|| std::env::var(name).ok()?.parse().ok())
        .flatten()
}
fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn timeout() -> Duration {
    let milliseconds = std::env::var("ACS_GPU_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (1..=300_000).contains(value))
        .unwrap_or(30_000);
    Duration::from_millis(milliseconds)
}
fn get_context(cancelled: &AtomicBool, kernel: Kernel) -> Result<&'static GpuContext, String> {
    start_initialization(kernel)?;
    let deadline = Instant::now() + timeout().max(Duration::from_secs(60));
    loop {
        check_wait(cancelled, deadline, "GPU 初始化超时")?;
        if let Some(context) = GPU_CONTEXTS[kernel.index()].get() {
            return context.as_ref().map_err(Clone::clone);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn start_initialization(kernel: Kernel) -> Result<(), String> {
    GPU_INITIALIZATIONS[kernel.index()]
        .get_or_init(|| {
            // Initialization may finish after cancellation or a GUI run change.
            // Keep its original attribution instead of borrowing a later run_id.
            let origin_run_id = GUI_DIAGNOSTICS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .map(|session| session.run_id.clone())
                .or_else(|| std::env::var("ACS_VALIDATION_RUN_ID").ok());
            std::thread::Builder::new()
                .name(format!("acs-gpu-{}-initialization", kernel.name()))
                .spawn(move || {
                    let started = Instant::now();
                    let initialized = std::panic::catch_unwind(|| {
                        pollster::block_on(GpuContext::new(kernel, &origin_run_id))
                    })
                    .unwrap_or_else(|_| Err("GPU 初始化发生异常".to_owned()));
                    if let Err(reason) = &initialized {
                        diagnostic(json!({"event":"initialization_failure", "reason":reason,
                            "total_ms":ms(started),"requested_kernel":kernel.name(),
                            "initialization_origin_run_id":origin_run_id}));
                    }
                    let _ = GPU_CONTEXTS[kernel.index()].set(initialized);
                })
                .map(|_| ())
                .map_err(|error| format!("无法启动 GPU 初始化：{error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
        .copied()
}
fn check_wait(cancelled: &AtomicBool, deadline: Instant, message: &str) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) {
        return Err("扫描已取消".to_owned());
    }
    if Instant::now() >= deadline {
        return Err(message.to_owned());
    }
    Ok(())
}
fn batch_size() -> Result<usize, String> {
    match std::env::var("ACS_GPU_BATCH") {
        Ok(value) => value
            .parse::<usize>()
            .ok()
            .filter(|value| (1..=MAX_BATCH_SIZE).contains(value))
            .ok_or_else(|| format!("ACS_GPU_BATCH 必须在 1..={MAX_BATCH_SIZE} 范围内")),
        Err(std::env::VarError::NotPresent) => Ok(DEFAULT_BATCH_SIZE),
        Err(error) => Err(format!("无法读取 ACS_GPU_BATCH：{error}")),
    }
}
fn stage_number(stage: ScoringStage) -> u32 {
    match stage {
        ScoringStage::RawLing6 => 0,
        ScoringStage::Postprocessed => 1,
    }
}
fn prepare_batch(seeds: &[i32]) -> PreparedBatch {
    let mut prepared = PreparedBatch {
        seeds: Vec::with_capacity(seeds.len()),
        inputs: Vec::with_capacity(seeds.len() * INPUT_WORDS),
        cpu_only: Vec::new(),
    };
    for &seed in seeds {
        prepared.seeds.push(seed);
        prepared.inputs.extend(Seed::new(seed).gpu_words(seed));
    }
    prepared
}
impl GpuContext {
    async fn new(kernel: Kernel, origin_run_id: &Option<String>) -> Result<Self, String> {
        let init_start = Instant::now();
        // Permit an explicit Vulkan/DX12 choice on AMD/Windows without changing
        // the generator, and record the chosen adapter below.
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|error| format!("未找到 GPU 适配器：{error}"))?;
        let info = adapter.get_info();
        eprintln!(
            "GPU adapter: {} | {:?} | {:?} | {} {}",
            info.name, info.backend, info.device_type, info.driver, info.driver_info
        );
        let adapter_limits = adapter.limits();
        diagnostic(
            json!({"event":"adapter", "name":info.name, "vendor":info.vendor,
            "initialization_origin_run_id":origin_run_id,
            "device":info.device, "device_type":format!("{:?}",info.device_type),
            "software":info.device_type == wgpu::DeviceType::Cpu,
            "backend":format!("{:?}",info.backend),"driver":info.driver,"driver_info":info.driver_info,
            "limits":format!("{adapter_limits:?}"),"features":format!("{:?}",adapter.features()),
            "discovery_ms":ms(init_start),"program_version":env!("CARGO_PKG_VERSION"),
            "kernel":kernel.name(),"shader_version":kernel.version(),
            "shader_fnv1a64":shader_hash(kernel),"debug_assertions":cfg!(debug_assertions),
            "shader_lf_fnv1a64":shader_lf_hash(kernel),"shader_hash_encoding":"rng file then generator file; LF hash normalizes CRLF",
            "diagnostics_version":"gpu-debug-v6", "timestamp_strategy_when_enabled":"separate_copy_encoder_one_submit",
            "rayon_num_threads":std::env::var("RAYON_NUM_THREADS").ok()}),
        );
        if info.device_type == wgpu::DeviceType::Cpu
            && std::env::var("ACS_GPU_ALLOW_SOFTWARE").as_deref() != Ok("1")
        {
            return Err(format!(
                "{} 是软件适配器；仅在测试时设置 ACS_GPU_ALLOW_SOFTWARE=1",
                info.name
            ));
        }
        let device_start = Instant::now();
        if kernel.cooperative() && adapter_limits.max_compute_workgroup_storage_size < 32768 {
            return Err(format!(
                "协作候选需要 32768 字节工作组内存，设备仅支持 {}",
                adapter_limits.max_compute_workgroup_storage_size
            ));
        }
        let required_limits = wgpu::Limits {
            max_compute_workgroup_storage_size: if kernel.cooperative() {
                32768
            } else {
                wgpu::Limits::default().max_compute_workgroup_storage_size
            },
            max_storage_buffer_binding_size: adapter_limits
                .max_storage_buffer_binding_size
                .min(256 * 1024 * 1024),
            max_buffer_size: adapter_limits.max_buffer_size.min(256 * 1024 * 1024),
            ..wgpu::Limits::default()
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits,
                required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
                ..wgpu::DeviceDescriptor::default()
            })
            .await
            .map_err(|error| format!("GPU 初始化失败：{error}"))?;
        let device_ms = ms(device_start);
        let coefficient_start = Instant::now();
        let coefficient_data =
            indexed_rng::gpu_coefficients().ok_or("GPU coefficient allocation failed")?;
        let coefficient_build_ms = ms(coefficient_start);
        let transfer_start = Instant::now();
        use wgpu::util::DeviceExt;
        let coefficients = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Exact indexed RNG coefficients"),
            contents: bytemuck::cast_slice(&coefficient_data),
            usage: wgpu::BufferUsages::STORAGE,
        });
        diagnostic(
            json!({"event":"coefficient_setup","kernel":kernel.name(),"build_ms":coefficient_build_ms,
            "initialization_origin_run_id":origin_run_id,
            "buffer_init_ms":ms(transfer_start),"bytes":coefficient_data.len()*16}),
        );
        drop(coefficient_data);
        let compile_start = Instant::now();
        let errors = Arc::new(Mutex::new(None));
        let callback_errors = Arc::clone(&errors);
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            diagnostic(json!({"event":"uncaptured_device_error", "kernel":kernel.name(),"reason":error.to_string()}));
            if let Ok(mut slot) = callback_errors.lock() {
                *slot = Some(error.to_string());
            }
        }));
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let (rng, generator) = kernel.sources();
        let source = format!("{rng}\n{generator}");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Exact ordered terrain generation"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Exact ordered terrain generation"),
            layout: None,
            module: &module,
            entry_point: Some("generate"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        if let Some(error) = validation.pop().await {
            return Err(format!("GPU 生成管线无效：{error}"));
        }
        diagnostic(
            json!({"event":"initialization", "kernel":kernel.name(),"device_ms":device_ms,
            "initialization_origin_run_id":origin_run_id,
            "shader_compile_ms":ms(compile_start),"total_ms":ms(init_start)}),
        );
        let context = Self {
            kernel,
            software: info.device_type == wgpu::DeviceType::Cpu,
            device,
            queue,
            pipeline,
            coefficients,
            errors,
            timestamp_state: Mutex::new(timing::TimestampState::default()),
            execution: Mutex::new(()),
            validated: Mutex::new(HashSet::new()),
        };
        context.check_health()?;
        Ok(context)
    }
    fn acquire(&self, cancelled: &AtomicBool) -> Result<MutexGuard<'_, ()>, String> {
        let deadline = Instant::now() + timeout().max(Duration::from_secs(60));
        loop {
            check_wait(cancelled, deadline, "GPU 正忙，等待扫描队列超时")?;
            match self.execution.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(TryLockError::Poisoned(_)) => return Err("GPU 扫描状态锁不可用".to_owned()),
                Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(2)),
            }
        }
    }
    fn check_health(&self) -> Result<(), String> {
        let errors = self
            .errors
            .lock()
            .map_err(|_| "GPU 错误状态锁不可用".to_owned())?;
        if let Some(error) = errors.as_ref() {
            Err(format!("GPU 设备错误：{error}"))
        } else {
            Ok(())
        }
    }
    fn mark_failed(&self, reason: &str) {
        if let Ok(mut error) = self.errors.lock() {
            *error = Some(reason.to_owned());
        }
    }
    fn buffers(
        &self,
        side: u32,
        requested: usize,
        timestamp_requested: bool,
    ) -> Result<ScanBuffers, String> {
        let allocation_start = Instant::now();
        self.check_health()?;
        let limits = self.device.limits();
        let binding_limit = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let cooperative = self.kernel.cooperative();
        let largest_map_buffer = if cooperative {
            u64::from(DIRS_CAPACITY) * 4
        } else {
            (u64::from(side) * u64::from(side) * 4).max(u64::from(DIRS_CAPACITY) * 4)
        };
        let device_capacity =
            usize::try_from(binding_limit / largest_map_buffer).unwrap_or(usize::MAX);
        let capacity = requested.min(device_capacity).min(
            limits.max_compute_workgroups_per_dimension as usize * if cooperative { 1 } else { 32 },
        );
        if capacity == 0 {
            return Err("当前地图尺寸超出 GPU 缓冲区上限".to_owned());
        }
        let maps = u64::try_from(capacity).map_err(|error| error.to_string())?;
        let create = |label, size, usage| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let config = create(
            "Map generation dimensions",
            32,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let inputs = create(
            "Seed magnitude and overflow-family descriptors",
            maps * 16,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let cells = (!cooperative).then(|| {
            create(
                "GPU-resident terrain cells",
                maps * u64::from(side) * u64::from(side) * 4,
                wgpu::BufferUsages::STORAGE,
            )
        });
        let dirs = create(
            "GPU-resident mine paths",
            maps * u64::from(DIRS_CAPACITY) * 4,
            wgpu::BufferUsages::STORAGE,
        );
        let bitmaps = (!cooperative).then(|| {
            create(
                "Terrain bitplanes and source snapshot",
                maps * u64::from((side * side).div_ceil(32)) * 17 * 4,
                wgpu::BufferUsages::STORAGE,
            )
        });
        let output = create(
            "Map count and completion status",
            maps * 16,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let readback = create(
            "Map count readback",
            maps * 16 + 16,
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        );
        let bound_buffers: Vec<&wgpu::Buffer> = if cooperative {
            vec![&config, &inputs, &dirs, &output, &self.coefficients]
        } else {
            vec![
                &config,
                &inputs,
                cells.as_ref().expect("v3 terrain"),
                &dirs,
                &output,
                &self.coefficients,
                bitmaps.as_ref().expect("v3 bitmaps"),
            ]
        };
        let entries = bound_buffers
            .iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect::<Vec<_>>();
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Resident generation buffers"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let timestamp_supported = self
            .device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY);
        let timestamps = (timestamp_requested && timestamp_supported).then(|| {
            (
                self.device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("Generation timestamps"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                }),
                create(
                    "Timestamp resolve",
                    256,
                    wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                ),
            )
        });
        diagnostic(
            json!({"event":"allocation", "ms":ms(allocation_start),"capacity":capacity,
            "kernel":self.kernel.name(),
            "terrain_bytes":if cooperative {0} else {maps*u64::from(side)*u64::from(side)*4},
            "path_bytes":maps*u64::from(DIRS_CAPACITY)*4,
            "bitmap_bytes":if cooperative {0} else {maps*u64::from((side*side).div_ceil(32))*17*4},
            "workgroup_storage_limit_bytes":if cooperative {32768} else {0},
            "timestamp_requested":timestamp_requested,"timestamp_supported":timestamp_supported,
            "timestamp_enabled":timestamps.is_some(),
            "shared_coefficient_bytes":32_000_000}),
        );
        // Bind-group ownership retains the two GPU-only buffers.
        Ok(ScanBuffers {
            config,
            inputs,
            output,
            readback,
            bind_group,
            capacity,
            timestamp_requested,
            timestamps,
        })
    }
    fn generate(
        &self,
        buffers: &ScanBuffers,
        inputs: &[u32],
        config: &ScanConfig,
        cancelled: &AtomicBool,
        purpose: &str,
    ) -> Result<Vec<GpuOutput>, String> {
        let generation_start = Instant::now();
        self.check_health()?;
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        if !inputs.len().is_multiple_of(INPUT_WORDS)
            || inputs.len() / INPUT_WORDS > buffers.capacity
        {
            return Err("GPU 生成输入尺寸不一致".to_owned());
        }
        let maps = u32::try_from(inputs.len() / INPUT_WORDS).map_err(|error| error.to_string())?;
        let side = u32::try_from(config.map_size).map_err(|error| error.to_string())?;
        let capacity = u32::try_from(buffers.capacity).map_err(|error| error.to_string())?;
        let uniform = [
            side,
            side,
            maps,
            capacity,
            stage_number(config.scoring_stage),
            DIRS_CAPACITY,
            if purpose == "scan" {
                test_option("ACS_GPU_TEST_DRAW_BUDGET")
                    .unwrap_or(MAX_DRAWS)
                    .min(MAX_DRAWS)
            } else {
                MAX_DRAWS
            },
            0,
        ];
        let dispatch_id = NEXT_DISPATCH_ID.fetch_add(1, Ordering::Relaxed);
        diagnostic(
            json!({"event":"dispatch_begin","kernel":self.kernel.name(),"dispatch_id":dispatch_id,
            "purpose":purpose,"maps":maps,"workgroups":self.kernel.workgroups(maps),
            "timestamp_requested":buffers.timestamp_requested,"timestamp_enabled":buffers.timestamps.is_some(),
            "side":side,"capacity":capacity,"draw_budget":uniform[6]}),
        );
        let upload_start = Instant::now();
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        self.queue
            .write_buffer(&buffers.config, 0, bytemuck::cast_slice(&uniform));
        self.queue
            .write_buffer(&buffers.inputs, 0, bytemuck::cast_slice(inputs));
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Map generation commands"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Generation measured pass"),
                timestamp_writes: buffers.timestamps.as_ref().map(|(query_set, _)| {
                    wgpu::ComputePassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: Some(1),
                    }
                }),
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &buffers.bind_group, &[]);
            pass.dispatch_workgroups(self.kernel.workgroups(maps), 1, 1);
        }
        let mapped_size = u64::from(maps) * 16;
        encoder.copy_buffer_to_buffer(&buffers.output, 0, &buffers.readback, 0, mapped_size);
        let mut timestamp_copy = None;
        if let Some((query_set, resolve)) = &buffers.timestamps {
            encoder.resolve_query_set(query_set, 0..2, resolve, 0);
            // Separate the query resolve from its buffer copy. This matches the
            // workaround in gfx-rs/wgpu#6406 for stale/zero AMD Vulkan queries.
            // Both encoders are submitted together: no extra CPU wait or dispatch.
            let mut copy_encoder =
                self.device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Resolved timestamp copy after generation encoder"),
                    });
            copy_encoder.copy_buffer_to_buffer(resolve, 0, &buffers.readback, mapped_size, 16);
            timestamp_copy = Some(copy_encoder.finish());
        }
        let generation_commands = encoder.finish();
        let submission_start = Instant::now();
        self.queue
            .submit(std::iter::once(generation_commands).chain(timestamp_copy));
        let submit_call_ms = ms(submission_start);
        let upload_encode_submit_ms = ms(upload_start);
        let wait_start = Instant::now();
        let scope_pop_start = Instant::now();
        let validation_error = pollster::block_on(validation.pop());
        let validation_scope_pop_ms = ms(scope_pop_start);
        if let Some(error) = validation_error {
            diagnostic(
                json!({"event":"dispatch_wait_failure","kernel":self.kernel.name(),
                "dispatch_id":dispatch_id,"purpose":purpose,"maps":maps,"stage":"validation_scope_pop",
                "validation_scope_pop_ms":validation_scope_pop_ms,"host_wait_ms":ms(wait_start),
                "total_ms":ms(generation_start),"reason":error.to_string()}),
            );
            return Err(format!("GPU 生成命令无效：{error}"));
        }
        let read_size = mapped_size + if buffers.timestamps.is_some() { 16 } else { 0 };
        let slice = buffers.readback.slice(..read_size);
        let (sender, receiver) = mpsc::channel();
        let map_register_start = Instant::now();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let callback_at = Instant::now();
            let _ = sender.send((result, callback_at));
        });
        let map_async_register_ms = ms(map_register_start);
        let mut wait_statistics = timing::WaitStatistics::default();
        let wait_loop_start = Instant::now();
        let callback_at = match self.wait_for_readback(&receiver, cancelled, &mut wait_statistics) {
            Ok(callback_at) => callback_at,
            Err(error) => {
                buffers.readback.unmap();
                diagnostic(
                    json!({"event":"dispatch_wait_failure", "kernel":self.kernel.name(),"dispatch_id":dispatch_id,
                "purpose":purpose,"maps":maps,"reason":error,
                "validation_scope_pop_ms":validation_scope_pop_ms,"map_async_register_ms":map_async_register_ms,
                "wait_loop_ms":ms(wait_loop_start),"wait_statistics":wait_statistics.details(),
                "host_wait_ms":ms(wait_start),"total_ms":ms(generation_start),
                "cancelled":cancelled.load(Ordering::Relaxed)}),
                );
                return Err(error);
            }
        };
        let wait_loop_ms = ms(wait_loop_start);
        let callback_to_wait_return_ms = ms(callback_at);
        let host_wait_ms = ms(wait_start);
        let submission_to_map_ms = ms(submission_start);
        let submission_to_map_callback_ms =
            callback_at.duration_since(submission_start).as_secs_f64() * 1000.0;
        let map_registration_to_callback_ms =
            callback_at.duration_since(map_register_start).as_secs_f64() * 1000.0;
        let readback_start = Instant::now();
        let data = slice.get_mapped_range();
        let timestamp_sample = buffers.timestamps.as_ref().map(|_| {
            let stamps = bytemuck::cast_slice::<u8, u64>(&data[mapped_size as usize..]);
            self.timestamp_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .inspect(
                    [stamps[0], stamps[1]],
                    f64::from(self.queue.get_timestamp_period()),
                    submission_to_map_ms,
                )
        });
        let device_execution_ms = timestamp_sample
            .as_ref()
            .and_then(|sample| sample.device_execution_ms);
        let output = bytemuck::cast_slice::<u8, u32>(&data[..mapped_size as usize])
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&[count, status, draws, queries]| GpuOutput {
                count: usize::try_from(count).unwrap_or(usize::MAX),
                status,
                draws,
                queries,
            })
            .collect::<Vec<_>>();
        drop(data);
        buffers.readback.unmap();
        self.check_health()?;
        diagnostic(
            json!({"event":"dispatch","purpose":purpose,"maps":maps,"dispatch_id":dispatch_id,
            "kernel":self.kernel.name(),"workgroups":self.kernel.workgroups(maps),
            "upload_bytes":inputs.len()*4+32,"download_bytes":read_size,
            "upload_encode_submit_ms":upload_encode_submit_ms,"host_wait_ms":host_wait_ms,
            "submit_call_ms":submit_call_ms,
            "host_wait_breakdown":{"validation_scope_pop_ms":validation_scope_pop_ms,
                "map_async_register_ms":map_async_register_ms,"wait_loop_ms":wait_loop_ms,
                "calls":wait_statistics.details(),
                "submission_to_map_callback_ms":submission_to_map_callback_ms,
                "map_registration_to_callback_ms":map_registration_to_callback_ms,
                "callback_to_wait_return_ms":callback_to_wait_return_ms},
            "host_timing_note":"scope pop, map registration, and wait loop are host_wait subspans; poll/channel calls are sequential subspans of wait loop; callback endpoints overlap the successful poll or map registration; submit_call is within upload_encode_submit; GPU time overlaps host spans. Do not sum overlapping spans.",
            "readback_decode_ms":ms(readback_start),"device_execution_ms":device_execution_ms,
            "submission_to_map_ms":submission_to_map_ms,
            "timestamp_diagnostic":timestamp_sample.as_ref().map(|sample|sample.details.clone()).unwrap_or_else(||json!({"status":if buffers.timestamp_requested {"unsupported"} else {"disabled_by_scan_options"},"scope":"No timestamp queries, resolves, or timestamp copies were encoded; generation and count readback are unchanged."})),
            "timestamp_strategy":if buffers.timestamps.is_some(){"separate_copy_encoder_one_submit"}else{"none"},
            "timestamp_requested":buffers.timestamp_requested,
            "timestamp_supported":self.device.features().contains(wgpu::Features::TIMESTAMP_QUERY),
            "timestamp_enabled":buffers.timestamps.is_some(),
            "timestamp_query_indices":if buffers.timestamps.is_some(){Some([0_u32,1_u32])}else{None},
            "total_ms":ms(generation_start),"successful":output.iter().filter(|x|x.status==0).count(),
            "budget_exceeded":output.iter().filter(|x|x.status==1).count(),
            "path_exceeded":output.iter().filter(|x|x.status==2).count(),
            "other_failures":output.iter().filter(|x|x.status>2).count(),
            "draws_total":output.iter().map(|x|u64::from(x.draws)).sum::<u64>(),
            "draws_max":output.iter().map(|x|x.draws).max(),
            "rng_queries":output.iter().map(|x|u64::from(x.queries)).sum::<u64>(),
            "rng_queries_meaning":"coefficient evaluations; cooperative sequential ring recurrence is excluded"}),
        );
        if output.len() != usize::try_from(maps).unwrap_or(0)
            || output.iter().any(|item| {
                item.status == 0
                    && (item.count > usize::try_from(side * side).unwrap_or(0)
                        || item.draws > MAX_DRAWS)
            })
        {
            return Err("GPU 生成返回了无效结果".to_owned());
        }
        Ok(output)
    }
    fn wait_for_readback(
        &self,
        receiver: &mpsc::Receiver<ReadbackSignal>,
        cancelled: &AtomicBool,
        statistics: &mut timing::WaitStatistics,
    ) -> Result<Instant, String> {
        let deadline = Instant::now() + timeout();
        loop {
            check_wait(cancelled, deadline, "GPU 读回超时")?;
            let poll_start = Instant::now();
            let polled = self.device.poll(wgpu::PollType::Poll);
            statistics.observe_poll(poll_start.elapsed());
            polled.map_err(|error| format!("GPU 轮询失败：{error}"))?;
            let channel_start = Instant::now();
            let received = receiver.recv_timeout(Duration::from_millis(2));
            statistics.observe_channel_wait(
                channel_start.elapsed(),
                matches!(&received, Err(mpsc::RecvTimeoutError::Timeout)),
            );
            match received {
                Ok((result, callback_at)) => {
                    return result
                        .map(|()| callback_at)
                        .map_err(|error| format!("GPU 读回失败：{error}"))
                }
                Err(mpsc::RecvTimeoutError::Timeout) => self.check_health()?,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("GPU 读回通道中断".to_owned())
                }
            }
        }
    }
    fn self_test(
        &self,
        buffers: &ScanBuffers,
        config: &ScanConfig,
        cancelled: &AtomicBool,
    ) -> Result<(), String> {
        let self_test_start = Instant::now();
        let key = (config.map_size, stage_number(config.scoring_stage));
        if self
            .validated
            .lock()
            .map_err(|_| "GPU 验证状态锁不可用".to_owned())?
            .contains(&key)
        {
            return Ok(());
        }
        let mut reference = MapMaker::try_new(0, config.map_size, config.map_size)
            .map_err(|error| error.to_string())?;
        reference.set_kernel(GenerationKernel::Frontier);
        for seeds in SELF_TEST_SEEDS.chunks(buffers.capacity) {
            if cancelled.load(Ordering::Relaxed) {
                return Err("扫描已取消".to_owned());
            }
            let prepared = prepare_batch(seeds);
            let output =
                self.generate(buffers, &prepared.inputs, config, cancelled, "self_test")?;
            for (&seed, item) in prepared.seeds.iter().zip(output) {
                reference.reset(seed);
                let expected = reference
                    .score_for_scan(config.scoring_stage, 0)
                    .ok_or_else(|| "CPU 自检未返回完整计数".to_owned())?;
                if item.status != 0 || item.count != expected {
                    return Err(format!(
                        "GPU 生成自检失败：种子 {seed}，CPU={expected}，GPU={}，状态={}",
                        item.count, item.status
                    ));
                }
            }
        }
        self.validated
            .lock()
            .map_err(|_| "GPU 验证状态锁不可用".to_owned())?
            .insert(key);
        diagnostic(
            json!({"event":"self_test","ms":ms(self_test_start),"map_size":key.0,"stage":key.1,"seeds":SELF_TEST_SEEDS.len()}),
        );
        Ok(())
    }
}

pub(super) fn scan(
    seeds: impl Iterator<Item = i32> + Send,
    config: &ScanConfig,
    progress: &AtomicUsize,
    cancelled: &AtomicBool,
    limit: Option<usize>,
    requested_kernel: Option<Kernel>,
    timestamp_requested: bool,
) -> (
    Vec<(i32, usize)>,
    BackendStatus,
    Option<Kernel>,
    Option<bool>,
) {
    let started = Instant::now();
    let before = progress.load(Ordering::Relaxed);
    diagnostic(json!({"event":"scan_begin", "map_size":config.map_size,
        "stage":stage_number(config.scoring_stage),"threshold":config.threshold,"top_k":limit,
        "requested_kernel":requested_kernel.map(|kernel|kernel.name().to_owned()).unwrap_or_else(||std::env::var("ACS_GPU_KERNEL").unwrap_or_else(|_|"v3".to_owned())),
        "timestamp_requested":timestamp_requested,
        "rayon_threads":rayon::current_num_threads()}));
    let mut actual_kernel = None;
    let mut actual_timestamps = None;
    let result = scan_inner(
        seeds,
        config,
        progress,
        cancelled,
        limit,
        requested_kernel,
        timestamp_requested,
        &mut actual_kernel,
        &mut actual_timestamps,
    );
    diagnostic(
        json!({"event":"scan_summary","total_ms":ms(started),"map_size":config.map_size,
        "stage":stage_number(config.scoring_stage),"threshold":config.threshold,"top_k":limit,
        "completed":progress.load(Ordering::Relaxed).saturating_sub(before),
        "matches":result.0.len(),"backend_status":format!("{:?}",result.1),
        "actual_kernel":actual_kernel.map(Kernel::name),
        "timestamp_requested":timestamp_requested,"timestamp_enabled":actual_timestamps,
        "cancelled":cancelled.load(Ordering::Relaxed)}),
    );
    (result.0, result.1, actual_kernel, actual_timestamps)
}
fn scan_inner(
    seeds: impl Iterator<Item = i32> + Send,
    config: &ScanConfig,
    progress: &AtomicUsize,
    cancelled: &AtomicBool,
    limit: Option<usize>,
    requested_kernel: Option<Kernel>,
    timestamp_requested: bool,
    actual_kernel: &mut Option<Kernel>,
    actual_timestamps: &mut Option<bool>,
) -> (Vec<(i32, usize)>, BackendStatus) {
    let mut seeds = seeds.peekable();
    if seeds.peek().is_none()
        || cancelled.load(Ordering::Relaxed)
        || config.validate().is_err()
        || limit == Some(0)
    {
        return (Vec::new(), BackendStatus::Cpu);
    }
    if config.map_size > MAX_GPU_SIDE {
        return cpu_fallback(
            seeds,
            config,
            progress,
            cancelled,
            limit,
            format!("GPU 精确生成支持边长 1..={MAX_GPU_SIDE}；当前尺寸使用 CPU"),
        );
    }
    let kernel = match requested_kernel.map_or_else(Kernel::selected, Ok) {
        Ok(kernel) => kernel,
        Err(reason) => return cpu_fallback(seeds, config, progress, cancelled, limit, reason),
    };
    let context = match get_context(cancelled, kernel) {
        Ok(context) => context,
        Err(reason) => return cpu_fallback(seeds, config, progress, cancelled, limit, reason),
    };
    *actual_kernel = Some(context.kernel);
    let _guard = match context.acquire(cancelled) {
        Ok(guard) => guard,
        Err(reason) => return cpu_fallback(seeds, config, progress, cancelled, limit, reason),
    };
    let buffers = batch_size().and_then(|capacity| {
        context.buffers(
            u32::try_from(config.map_size).map_err(|error| error.to_string())?,
            capacity,
            timestamp_requested,
        )
    });
    let buffers = match buffers {
        Ok(buffers) => buffers,
        Err(reason) => return cpu_fallback(seeds, config, progress, cancelled, limit, reason),
    };
    *actual_timestamps = Some(buffers.timestamps.is_some());
    if let Err(reason) = context.self_test(&buffers, config, cancelled) {
        if !cancelled.load(Ordering::Relaxed) {
            context.mark_failed(&reason);
        }
        return cpu_fallback(seeds, config, progress, cancelled, limit, reason);
    }
    scan_batches(seeds, config, progress, cancelled, limit, context, &buffers)
}
fn cpu_fallback(
    seeds: impl Iterator<Item = i32> + Send,
    config: &ScanConfig,
    progress: &AtomicUsize,
    cancelled: &AtomicBool,
    limit: Option<usize>,
    reason: String,
) -> (Vec<(i32, usize)>, BackendStatus) {
    let before = progress.load(Ordering::Relaxed);
    let start = Instant::now();
    let matches = cpu::scan_seed_iterator_limited(seeds, config, progress, cancelled, limit);
    diagnostic(
        json!({"event":"cpu_fallback","reason":reason,"ms":ms(start),
        "gpu_completed":0,"cpu_fallback_completed":progress.load(Ordering::Relaxed).saturating_sub(before)}),
    );
    (matches, BackendStatus::CpuFallback(reason))
}
fn scan_batches(
    mut seeds: impl Iterator<Item = i32> + Send,
    config: &ScanConfig,
    progress: &AtomicUsize,
    cancelled: &AtomicBool,
    limit: Option<usize>,
    context: &GpuContext,
    buffers: &ScanBuffers,
) -> (Vec<(i32, usize)>, BackendStatus) {
    let mut matches = cpu::MatchCollector::new(limit);
    let mut cpu_seeds = 0;
    let mut gpu_seeds = 0;
    let mut fallback_reason = String::new();
    let mut batch_index = 0u32;
    while !cancelled.load(Ordering::Relaxed) {
        let batch: Vec<_> = seeds.by_ref().take(buffers.capacity).collect();
        if batch.is_empty() {
            break;
        }
        let prepare_start = Instant::now();
        let prepared = prepare_batch(&batch);
        diagnostic(
            json!({"event":"input_prepare","ms":ms(prepare_start),"batch_index":batch_index,"seeds":batch.len(),"cpu_only":prepared.cpu_only.len()}),
        );
        // Explicit test-only fault occurs after real dispatch/readback.
        let generated = context
            .generate(buffers, &prepared.inputs, config, cancelled, "scan")
            .and_then(|output| {
                if test_option("ACS_GPU_TEST_FAIL_BATCH") == Some(batch_index) {
                    Err("explicit test-mode post-dispatch failure".to_owned())
                } else {
                    Ok(output)
                }
            });
        batch_index += 1;
        let output = match generated {
            Ok(output) => output,
            Err(_) if cancelled.load(Ordering::Relaxed) => break,
            Err(reason) => {
                diagnostic(
                    json!({"event":"dispatch_failure","reason":reason,"batch_index":batch_index-1}),
                );
                context.mark_failed(&reason);
                let fallback_start = Instant::now();
                // This batch is still uncommitted. Restart all its inputs and
                // the remaining iterator; preserve duplicate seeds as inputs.
                let before = progress.load(Ordering::Relaxed);
                matches.extend(cpu::scan_seed_iterator_limited(
                    batch.into_iter().chain(seeds),
                    config,
                    progress,
                    cancelled,
                    limit,
                ));
                let recovered = progress.load(Ordering::Relaxed).saturating_sub(before);
                diagnostic(
                    json!({"event":"batch_cpu_fallback","ms":ms(fallback_start),"completed":recovered,"reason":reason}),
                );
                cpu_seeds += recovered;
                fallback_reason = reason;
                break;
            }
        };
        let commit_start = Instant::now();
        let (gpu_completed, cpu_completed, reason) = commit_batch(
            prepared,
            output,
            config,
            progress,
            cancelled,
            limit,
            &mut matches,
        );
        diagnostic(
            json!({"event":"commit","ms":ms(commit_start),"gpu_completed":gpu_completed,"cpu_fallback_completed":cpu_completed,"reason":reason}),
        );
        gpu_seeds += gpu_completed;
        cpu_seeds += cpu_completed;
        if let Some(reason) = reason {
            fallback_reason = reason;
        }
    }
    let status = if gpu_seeds == 0 && !fallback_reason.is_empty() {
        BackendStatus::CpuFallback(fallback_reason.clone())
    } else if context.software {
        BackendStatus::SoftwareGpu {
            cpu_fallback_seeds: cpu_seeds,
            reason: fallback_reason.clone(),
        }
    } else if fallback_reason.is_empty() {
        BackendStatus::Gpu
    } else if gpu_seeds == 0 {
        BackendStatus::CpuFallback(fallback_reason.clone())
    } else {
        BackendStatus::GpuWithCpuFallback {
            seeds: cpu_seeds,
            reason: fallback_reason.clone(),
        }
    };
    let sort_start = Instant::now();
    let sorted = matches.into_sorted_matches();
    diagnostic(
        json!({"event":"scan_execution","gpu_completed":gpu_seeds,"cpu_fallback_completed":cpu_seeds,
        "sort_ms":ms(sort_start),"reason":fallback_reason,"software":context.software}),
    );
    (sorted, status)
}
fn commit_batch(
    prepared: PreparedBatch,
    output: Vec<GpuOutput>,
    config: &ScanConfig,
    progress: &AtomicUsize,
    cancelled: &AtomicBool,
    limit: Option<usize>,
    matches: &mut cpu::MatchCollector,
) -> (usize, usize, Option<String>) {
    let mut retry = prepared.cpu_only;
    let mut reason = (!retry.is_empty()).then(|| "非规范初始随机状态".to_owned());
    let mut gpu_completed = 0;
    for (seed, item) in prepared.seeds.into_iter().zip(output) {
        if item.status != 0 {
            diagnostic(json!({"event":"seed_failure", "seed":seed,
                "status":item.status,"logical_draws":item.draws,
                "coefficient_queries":item.queries,
                "reason":match item.status {1=>"draw_budget_exceeded",2=>"path_capacity_exceeded",3=>"unsupported_random_span",_=>"unknown_shader_status"}}));
            retry.push(seed);
            reason = Some(format!("GPU 种子预算或内核状态 {}", item.status));
            continue;
        }
        // Thresholds never suppress generation or RNG-dependent work.
        if item.count >= config.threshold {
            matches.extend([(seed, item.count)]);
        }
        progress.fetch_add(1, Ordering::Relaxed);
        gpu_completed += 1;
    }
    let before = progress.load(Ordering::Relaxed);
    if !retry.is_empty() {
        matches.extend(cpu::scan_seed_list_limited(
            retry, config, progress, cancelled, limit,
        ));
    }
    (
        gpu_completed,
        progress.load(Ordering::Relaxed).saturating_sub(before),
        reason,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_uploads_exact_indexed_seed_descriptors() {
        let seeds = [0, 1, -1, 10_000, 161_803_399, i32::MIN, i32::MAX, 10_000];
        let prepared = prepare_batch(&seeds);
        assert_eq!(prepared.cpu_only, Vec::<i32>::new());
        assert_eq!(prepared.seeds, seeds);
        assert_eq!(prepared.inputs.len(), INPUT_WORDS * seeds.len());
        for (&seed, record) in seeds
            .iter()
            .zip(prepared.inputs.as_chunks::<INPUT_WORDS>().0.iter())
        {
            let [record_seed, ring @ ..] = record;
            assert_eq!(*record_seed, seed as u32);
            assert_eq!(*ring, Seed::new(seed).gpu_words(seed)[1..]);
        }
    }
    #[test]
    fn unsupported_gpu_dimensions_recompute_all_seeds_on_cpu() {
        let config = ScanConfig {
            map_size: MAX_GPU_SIDE + 1,
            threshold: 0,
            scoring_stage: ScoringStage::RawLing6,
        };
        let seeds = vec![1, -1, 1];
        let progress = AtomicUsize::new(0);
        let cancelled = AtomicBool::new(false);
        let expected = cpu::scan_seed_list_limited(
            seeds.clone(),
            &config,
            &AtomicUsize::new(0),
            &cancelled,
            None,
        );
        let (actual, status, kernel, actual_timestamps) = scan(
            seeds.into_iter(),
            &config,
            &progress,
            &cancelled,
            None,
            Some(Kernel::Cooperative),
            true,
        );
        assert_eq!(actual, expected);
        assert_eq!(progress.load(Ordering::Relaxed), 3);
        assert!(matches!(status, BackendStatus::CpuFallback(_)));
        assert_eq!(kernel, None);
        assert_eq!(actual_timestamps, None);
    }
    #[test]
    fn failed_gpu_seed_is_recomputed_once_and_merged_with_duplicates() {
        let config = ScanConfig {
            map_size: 33,
            threshold: 0,
            scoring_stage: ScoringStage::Postprocessed,
        };
        let cancelled = AtomicBool::new(false);
        let all_seeds = vec![10_000, 10_001, 10_000, 10_002];
        let all =
            cpu::scan_seed_list_limited(all_seeds, &config, &AtomicUsize::new(0), &cancelled, None);
        let count = all
            .iter()
            .find(|item| item.0 == 10_000)
            .map_or(usize::MAX, |item| item.1);
        for limit in [None, Some(1), Some(3)] {
            let mut prepared = prepare_batch(&[10_000, 10_001, 10_000]);
            prepared.cpu_only.push(10_002);
            // A failed output's count must never be observed, even if enormous.
            let output = vec![
                GpuOutput {
                    count,
                    status: 0,
                    draws: 1,
                    queries: 1,
                },
                GpuOutput {
                    count: usize::MAX,
                    status: 2,
                    draws: 1,
                    queries: 1,
                },
                GpuOutput {
                    count,
                    status: 0,
                    draws: 1,
                    queries: 1,
                },
            ];
            let progress = AtomicUsize::new(0);
            let mut matches = cpu::MatchCollector::new(limit);
            let (gpu, cpu, reason) = commit_batch(
                prepared,
                output,
                &config,
                &progress,
                &cancelled,
                limit,
                &mut matches,
            );
            assert_eq!((gpu, cpu), (2, 2));
            assert!(reason.is_some());
            assert_eq!(progress.load(Ordering::Relaxed), 4);
            assert_eq!(
                matches.into_sorted_matches(),
                all.iter()
                    .copied()
                    .take(limit.unwrap_or(usize::MAX))
                    .collect::<Vec<_>>()
            );
        }
        assert!(debug_diagnostics_snapshot()
            .iter()
            .any(|event| event["event"] == "seed_failure"
                && event["seed"] == 10_001
                && event["status"] == 2));
    }
}
