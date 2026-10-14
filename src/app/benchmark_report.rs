use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub(super) struct BenchmarkReport {
    directory: PathBuf,
    metadata: Value,
    stages: Vec<Value>,
    runs: Vec<Value>,
    started: Instant,
    capture_active: bool,
    finished: bool,
}

fn io_error(path: &Path, error: impl std::fmt::Display) -> String {
    format!("无法写入诊断报告 {}：{error}", path.display())
}

fn create_report_directory(kind: &str) -> Result<PathBuf, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("系统时间无效：{error}"))?
        .as_millis();
    let mut roots = Vec::new();
    let category = if kind == "debug" {
        "debug-logs"
    } else {
        "benchmarks"
    };
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("ACSSeedFinder").join(category));
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            roots.push(parent.join(if kind == "debug" {
                "debug-logs"
            } else {
                "benchmark-reports"
            }));
        }
    }
    roots.push(std::env::temp_dir().join("ACSSeedFinder").join(category));
    let mut failures = Vec::new();
    for root in roots {
        if let Err(error) = fs::create_dir_all(&root) {
            failures.push(io_error(&root, error));
            continue;
        }
        for suffix in 0..100 {
            let directory = root.join(format!(
                "{kind}-{timestamp}-{}-{suffix}",
                std::process::id()
            ));
            match fs::create_dir(&directory) {
                Ok(()) => return Ok(directory),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    failures.push(io_error(&directory, error));
                    break;
                }
            }
        }
    }
    Err(format!("无法创建自动报告目录：{}", failures.join("；")))
}

fn file_identity(path: &Path) -> Result<Value, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    let mut reader = BufReader::new(file);
    let mut digest = Sha1::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(json!({"path":path, "bytes":size, "sha1":format!("{:x}", digest.finalize())}))
}

fn executable_identity() -> Value {
    std::env::current_exe()
        .map_err(|error| error.to_string())
        .and_then(|path| file_identity(&path))
        .unwrap_or_else(|error| json!({"identity_error":error}))
}

// Retain compatibility with toolchains where CPUID intrinsics require unsafe.
#[allow(unused_unsafe)]
fn cpu_brand() -> Option<String> {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        #[cfg(target_arch = "x86")]
        use std::arch::x86::__cpuid;
        #[cfg(target_arch = "x86_64")]
        use std::arch::x86_64::__cpuid;
        // SAFETY: supported Rust x86/x86_64 targets provide CPUID. Check the
        // maximum extended leaf before reading the optional brand-string leaves.
        let maximum = unsafe { __cpuid(0x8000_0000) }.eax;
        if maximum < 0x8000_0004 {
            return None;
        }
        let mut bytes = Vec::with_capacity(48);
        for leaf in 0x8000_0002..=0x8000_0004 {
            // SAFETY: the extended leaf range was checked above.
            let value = unsafe { __cpuid(leaf) };
            for register in [value.eax, value.ebx, value.ecx, value.edx] {
                bytes.extend_from_slice(&register.to_le_bytes());
            }
        }
        let brand = String::from_utf8_lossy(&bytes)
            .trim_matches(|character: char| character == '\0' || character.is_whitespace())
            .to_owned();
        (!brand.is_empty()).then_some(brand)
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    None
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let file = File::create(path).map_err(|error| io_error(path, error))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value).map_err(|error| io_error(path, error))?;
    writer.flush().map_err(|error| io_error(path, error))
}

fn write_events(path: &Path, events: &[Value]) -> Result<(), String> {
    let file = File::create(path).map_err(|error| io_error(path, error))?;
    let mut writer = BufWriter::new(file);
    for event in events {
        serde_json::to_writer(&mut writer, event).map_err(|error| io_error(path, error))?;
        writeln!(writer).map_err(|error| io_error(path, error))?;
    }
    writer.flush().map_err(|error| io_error(path, error))
}

impl BenchmarkReport {
    pub(super) fn new(input: Value) -> Result<Self, String> {
        let directory = create_report_directory("benchmark")?;
        let requested_environment = [
            "ACS_GPU_KERNEL",
            "ACS_GPU_TIMESTAMPS",
            "ACS_GPU_BATCH",
            "ACS_GPU_ALLOW_SOFTWARE",
            "ACS_GPU_TIMEOUT_MS",
            "ACS_GPU_DIAGNOSTICS",
            "WGPU_BACKEND",
            "WGPU_DX12_COMPILER",
            "RAYON_NUM_THREADS",
        ]
        .into_iter()
        .map(|name| (name, std::env::var(name).ok()))
        .collect::<std::collections::BTreeMap<_, _>>();
        let metadata = json!({
            "schema_version":1,
            "kind":"gui_benchmark",
            "started_unix_milliseconds":SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_millis()),
            "report_directory":directory,
            "input":input,
            "program_version":env!("CARGO_PKG_VERSION"),
            "build_profile":if cfg!(debug_assertions) {"debug"} else {"release"},
            "compile_git_sha":option_env!("GITHUB_SHA"),
            "compile_rustflags":option_env!("RUSTFLAGS"),
            "compile_build_info":include_str!(concat!(env!("OUT_DIR"), "/build-info.txt")),
            "resolved_wgpu_version":"unknown",
            "dependency_version_note":"Exact wgpu patch version is not inferred from Cargo.lock. Research CI dependency metadata/tree records can establish the resolved version for that build.",
            "executable":executable_identity(),
            "host_os":std::env::consts::OS,
            "host_arch":std::env::consts::ARCH,
            "processor_identifier":std::env::var("PROCESSOR_IDENTIFIER").ok(),
            "cpu_brand":cpu_brand(),
            "rayon_threads":rayon::current_num_threads(),
            "available_parallelism":std::thread::available_parallelism().ok().map(|count| count.get()),
            "requested_environment":requested_environment,
            "gpu_feature_enabled":cfg!(feature="gpu"),
            "pure_hardware_gpu_definition":"Per-run pure_hardware_gpu is true only when backend is Gpu and every requested seed completed without cancellation. Count equality and requested-kernel identity are separate checks; inspect exact_equal, kernel_expectation_pass and validation_passed.",
            "timestamp_comparison_definition":"requested_gpu_timestamps is explicit per GPU candidate; actual_gpu_timestamps comes from scan buffers and may be false when timestamp features are unavailable. A requested/actual mismatch suppresses the GPU performance ratio, while exact_equal and validation_passed retain count/kernel correctness evidence. Disabled timestamps are not CPU fallback.",
            "external_diagnostics_enabled":std::env::var_os("ACS_GPU_DIAGNOSTICS").is_some(),
            "timing_method":"Instant around each complete scanner call; includes filtering, sorting and any CPU fallback. Warmup, equality checks, result export and this report's diagnostic JSONL export are outside those measured intervals. In-memory diagnostic collection occurs during calls. If ACS_GPU_DIAGNOSTICS was explicitly set, its additional external JSONL writes remain enabled and occur inside scan timing. Host wait overlaps device time and must not be added to it.",
            "evidence_scope":"One warmed scan per candidate/workload; not repeated hardware acceptance. Actual adapter, backend, driver, shader hash and kernel are authoritative in GPU diagnostic events; an environment request alone does not establish actual execution."
        });
        write_json(&directory.join("metadata.json"), &metadata)?;
        #[cfg(feature = "gpu")]
        crate::scanner::begin_gui_diagnostics().map_err(|error| {
            format!(
                "无法开始 GPU 诊断会话：{error}；报告目录：{}",
                directory.display()
            )
        })?;
        Ok(Self {
            directory,
            metadata,
            stages: Vec::new(),
            runs: Vec::new(),
            started: Instant::now(),
            capture_active: cfg!(feature = "gpu"),
            finished: false,
        })
    }

    pub(super) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(super) fn set_run_id(&self, run_id: &str) {
        #[cfg(feature = "gpu")]
        crate::scanner::set_gui_diagnostic_run_id(run_id);
        #[cfg(not(feature = "gpu"))]
        let _ = run_id;
    }

    pub(super) fn record_stage(&mut self, stage: Value) {
        self.stages.push(stage);
    }

    pub(super) fn record_run(
        &mut self,
        run_id: &str,
        mut row: Value,
        counts: &[(i32, usize)],
    ) -> Result<(), String> {
        row["run_id"] = json!(run_id);
        row["counts_file"] = json!(format!("{run_id}-counts.csv"));
        self.runs.push(row.clone());
        let counts_path = self.directory.join(format!("{run_id}-counts.csv"));
        let mut counts_writer = BufWriter::new(
            File::create(&counts_path).map_err(|error| io_error(&counts_path, error))?,
        );
        writeln!(counts_writer, "rank,seed,count")
            .map_err(|error| io_error(&counts_path, error))?;
        for (index, (seed, count)) in counts.iter().enumerate() {
            writeln!(counts_writer, "{},{seed},{count}", index + 1)
                .map_err(|error| io_error(&counts_path, error))?;
        }
        counts_writer
            .flush()
            .map_err(|error| io_error(&counts_path, error))?;
        let raw_path = self.directory.join("runs.jsonl");
        let mut raw = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&raw_path)
            .map_err(|error| io_error(&raw_path, error))?;
        serde_json::to_writer(&mut raw, &row).map_err(|error| io_error(&raw_path, error))?;
        writeln!(raw).map_err(|error| io_error(&raw_path, error))
    }

    fn end_capture(&mut self) -> Vec<Value> {
        if self.capture_active {
            self.capture_active = false;
            #[cfg(feature = "gpu")]
            return crate::scanner::end_gui_diagnostics();
        }
        Vec::new()
    }

    pub(super) fn finish(&mut self, error: Option<&str>, cancelled: bool) -> Result<(), String> {
        let events = self.end_capture();
        let events_path = self.directory.join("gpu-diagnostics.jsonl");
        let events_result = write_events(&events_path, &events);
        let csv_result = self.write_csv();
        let mut export_errors = events_result
            .err()
            .into_iter()
            .chain(csv_result.err())
            .collect::<Vec<_>>();
        let gpu_metadata = events
            .iter()
            .filter(|event| {
                event.get("shader_fnv1a64").is_some()
                    || event.get("kernel").is_some()
                    || event.get("actual_kernel").is_some()
                    || event
                        .get("event")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| kind.contains("adapter"))
            })
            .collect::<Vec<_>>();
        let report = json!({
            "metadata":self.metadata,
            "state":if error.is_some() || !export_errors.is_empty() {"failed"} else if cancelled {"cancelled"} else {"complete"},
            "error":error,
            "cancelled":cancelled,
            "wall_seconds_after_metadata":self.started.elapsed().as_secs_f64(),
            "setup_and_warmup":self.stages,
            "runs":self.runs,
            "gpu_metadata":gpu_metadata,
            "gpu_diagnostic_events":events.len(),
            "gpu_diagnostics_file":"gpu-diagnostics.jsonl",
            "export_errors":export_errors,
        });
        if let Err(error) = write_json(&self.directory.join("report.json"), &report) {
            export_errors.push(error);
        }
        self.finished = true;
        if export_errors.is_empty() {
            Ok(())
        } else {
            Err(export_errors.join("；"))
        }
    }

    fn write_csv(&self) -> Result<(), String> {
        let path = self.directory.join("runs.csv");
        let mut writer =
            BufWriter::new(File::create(&path).map_err(|error| io_error(&path, error))?);
        let columns = [
            "run_id",
            "requested_mode",
            "candidate",
            "requested_kernel",
            "actual_kernel",
            "requested_gpu_timestamps",
            "actual_gpu_timestamps",
            "experimental_candidate",
            "threshold",
            "map_size",
            "stage",
            "seed_start",
            "seed_end",
            "input_seeds",
            "completed_seeds",
            "seconds",
            "returned_items",
            "returned_count_sum",
            "exact_equal",
            "kernel_expectation_pass",
            "timestamp_expectation_pass",
            "validation_passed",
            "pure_hardware_gpu",
            "gpu_performance_eligible",
            "backend_status",
            "counts_file",
        ];
        writeln!(writer, "{}", columns.join(",")).map_err(|error| io_error(&path, error))?;
        for row in &self.runs {
            let fields = columns
                .iter()
                .map(|name| {
                    let value = &row[*name];
                    let text = value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned);
                    format!("\"{}\"", text.replace('"', "\"\""))
                })
                .collect::<Vec<_>>();
            writeln!(writer, "{}", fields.join(",")).map_err(|error| io_error(&path, error))?;
        }
        writer.flush().map_err(|error| io_error(&path, error))
    }
}

impl Drop for BenchmarkReport {
    fn drop(&mut self) {
        if !self.finished {
            // Preserve already completed rows and release the capture if a
            // background-task panic unwinds through the benchmark.
            let _ = self.finish(
                Some("benchmark interrupted before normal finalization"),
                true,
            );
        }
    }
}

pub(super) fn export_debug_log(ui_context: Value) -> Result<PathBuf, String> {
    let directory = create_report_directory("debug")?;
    #[cfg(feature = "gpu")]
    let events = crate::scanner::debug_diagnostics_snapshot();
    #[cfg(not(feature = "gpu"))]
    let events: Vec<Value> = Vec::new();
    let gpu_metadata = events
        .iter()
        .filter(|event| {
            event.get("shader_fnv1a64").is_some()
                || event.get("kernel").is_some()
                || event.get("actual_kernel").is_some()
                || event
                    .get("event")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind.contains("adapter"))
        })
        .collect::<Vec<_>>();
    let report = json!({
        "schema_version":1,
        "kind":"debug_snapshot",
        "created_unix_milliseconds":SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_millis()),
        "program_version":env!("CARGO_PKG_VERSION"),
        "build_profile":if cfg!(debug_assertions) {"debug"} else {"release"},
        "compile_git_sha":option_env!("GITHUB_SHA"),
        "compile_rustflags":option_env!("RUSTFLAGS"),
        "compile_build_info":include_str!(concat!(env!("OUT_DIR"), "/build-info.txt")),
        "resolved_wgpu_version":"unknown",
        "dependency_version_note":"Exact wgpu patch version is not inferred from Cargo.lock. Research CI dependency metadata/tree records can establish the resolved version for that build.",
        "executable":executable_identity(),
        "host_os":std::env::consts::OS,
        "host_arch":std::env::consts::ARCH,
        "processor_identifier":std::env::var("PROCESSOR_IDENTIFIER").ok(),
        "cpu_brand":cpu_brand(),
        "rayon_threads":rayon::current_num_threads(),
        "available_parallelism":std::thread::available_parallelism().ok().map(|count|count.get()),
        "requested_gpu_kernel":std::env::var("ACS_GPU_KERNEL").ok(),
        "requested_gpu_timestamps_environment":std::env::var("ACS_GPU_TIMESTAMPS").ok(),
        "requested_wgpu_backend":std::env::var("WGPU_BACKEND").ok(),
        "requested_dx12_compiler":std::env::var("WGPU_DX12_COMPILER").ok(),
        "external_diagnostics_enabled":std::env::var_os("ACS_GPU_DIAGNOSTICS").is_some(),
        "gpu_feature_enabled":cfg!(feature="gpu"),
        "ui_context":ui_context,
        "gpu_metadata":gpu_metadata,
        "recent_event_count":events.len(),
        "recent_events":events,
        "recent_events_file":"gpu-diagnostics.jsonl",
        "share_file":"debug-report.json contains the device identity, recent stage events and errors; this single file is sufficient to share the debug snapshot.",
        "evidence_scope":"Snapshot of bounded recent diagnostic events and cached device identity. Earlier events may have been evicted. This is not a complete benchmark, correctness proof, or hardware acceptance. Exporting does not initialize or dispatch a GPU."
    });
    write_events(&directory.join("gpu-diagnostics.jsonl"), &events)?;
    write_json(&directory.join("debug-report.json"), &report)?;
    Ok(directory)
}

pub(super) fn open_directory(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let command = "explorer.exe";
    #[cfg(target_os = "macos")]
    let command = "open";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let command = "xdg-open";
    std::process::Command::new(command)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开报告目录 {}：{error}", path.display()))
}

// These tests are deliberately available only in a build with GPU support
// absent. They exercise real report-file export with labelled synthetic rows,
// never a device, shader, terrain generator, or performance measurement.
#[cfg(all(test, not(feature = "gpu")))]
mod tests {
    use super::*;

    fn read_json(path: &Path) -> Result<Value, Box<dyn std::error::Error>> {
        Ok(serde_json::from_reader(File::open(path)?)?)
    }

    #[test]
    fn exports_ten_rows_partial_failure_cancel_and_debug_without_gpu(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut report =
            BenchmarkReport::new(json!({"synthetic_writer_test":true,"no_dispatch":true}))?;
        let directory = report.directory().to_owned();
        let candidates = [
            "cpu",
            "v3",
            "cooperative",
            "cooperative-no-timestamps",
            "cooperative_words",
        ];
        let kernels = [
            None,
            Some("v3"),
            Some("cooperative"),
            Some("cooperative"),
            Some("cooperative_words"),
        ];
        let timestamps = [None, Some(true), Some(true), Some(false), Some(false)];
        for (index, threshold) in [0, 0, 0, 0, 0, 600, 600, 600, 600, 600]
            .into_iter()
            .enumerate()
        {
            let candidate_index = index % candidates.len();
            let id = format!("writer-case-{index}");
            let counts = if threshold == 0 {
                vec![(10000, 650), (10001, 400)]
            } else {
                vec![(10000, 650)]
            };
            report.record_run(
                &id,
                json!({
                    "synthetic":true, "synthetic_writer_test":true, "seconds_are_synthetic":true,
                    "requested_mode":if candidate_index == 0 {"cpu"} else {"gpu"},
                    "candidate":candidates[candidate_index],
                    "requested_kernel":kernels[candidate_index],
                    "actual_kernel":kernels[candidate_index],
                    "requested_gpu_timestamps":timestamps[candidate_index],
                    "actual_gpu_timestamps":timestamps[candidate_index],
                    "experimental_candidate":candidate_index >= 2,
                    "threshold":threshold, "map_size":192, "stage":"Postprocessed",
                    "seed_start":10000, "seed_end":10001, "input_seeds":2, "completed_seeds":2,
                    "seconds":0.125 * (index + 1) as f64, "returned_items":counts.len(),
                    "returned_count_sum":counts.iter().map(|(_,count)|*count).sum::<usize>(),
                    "complete":true, "exact_equal":true, "pure_hardware_gpu":false,
                    "backend_status":"Synthetic writer fixture; GPU feature absent; no dispatch"
                }),
                &counts,
            )?;
        }
        report.finish(None, false)?;
        let saved = read_json(&directory.join("report.json"))?;
        assert_eq!(saved["state"], "complete");
        assert_eq!(saved["runs"].as_array().map(Vec::len), Some(10));
        assert_eq!(saved["metadata"]["gpu_feature_enabled"], false);
        assert_eq!(saved["metadata"]["input"]["no_dispatch"], true);
        let build_info = saved["metadata"]["compile_build_info"]
            .as_str()
            .ok_or("missing embedded build information")?;
        for field in [
            "TARGET=",
            "PROFILE=",
            "OPT_LEVEL=",
            "DEBUG=",
            "rustc_version_status=",
        ] {
            assert!(build_info.contains(field));
        }
        assert_eq!(saved["metadata"]["resolved_wgpu_version"], "unknown");
        let identity = &saved["metadata"]["executable"];
        // Restricted Linux test environments can hide /proc/self/exe. That
        // case must export an explicit identity error, not a made-up hash.
        assert!(
            identity["sha1"]
                .as_str()
                .is_some_and(|hash| hash.len() == 40)
                || identity["identity_error"]
                    .as_str()
                    .is_some_and(|error| !error.is_empty())
        );
        let fixture = directory.join("synthetic-identity-fixture.txt");
        fs::write(&fixture, b"abc")?;
        assert_eq!(
            file_identity(&fixture)?["sha1"],
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        let csv = fs::read_to_string(directory.join("runs.csv"))?;
        let csv_lines = csv.lines().collect::<Vec<_>>();
        assert_eq!(csv_lines.len(), 11);
        let csv_columns = csv_lines[0].split(',').collect::<Vec<_>>();
        let raw = fs::read_to_string(directory.join("runs.jsonl"))?
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(raw.len(), 10);
        for (index, row) in raw.iter().enumerate() {
            let candidate_index = index % candidates.len();
            assert_eq!(row["seconds"], json!(0.125 * (index + 1) as f64));
            assert_eq!(row["threshold"], if index < 5 { 0 } else { 600 });
            assert_eq!(row["synthetic"], true);
            assert_eq!(row["pure_hardware_gpu"], false);
            assert_eq!(row, &saved["runs"][index]);
            // These synthetic CSV fixture fields deliberately contain no commas.
            let csv_fields = csv_lines[index + 1].split(',').collect::<Vec<_>>();
            assert_eq!(csv_fields.len(), csv_columns.len());
            for (name, expected) in [
                ("candidate", json!(candidates[candidate_index])),
                ("requested_kernel", json!(kernels[candidate_index])),
                ("actual_kernel", json!(kernels[candidate_index])),
                (
                    "requested_gpu_timestamps",
                    json!(timestamps[candidate_index]),
                ),
                ("actual_gpu_timestamps", json!(timestamps[candidate_index])),
                ("experimental_candidate", json!(candidate_index >= 2)),
            ] {
                assert_eq!(row[name], expected);
                let column = csv_columns
                    .iter()
                    .position(|column| *column == name)
                    .ok_or("missing candidate CSV column")?;
                let expected_text = expected
                    .as_str()
                    .map_or_else(|| expected.to_string(), str::to_owned);
                assert_eq!(csv_fields[column], format!("\"{expected_text}\""));
            }
            let count_file = row["counts_file"].as_str().ok_or("missing counts file")?;
            let expected_lines = if index < 5 { 3 } else { 2 };
            assert_eq!(
                fs::read_to_string(directory.join(count_file))?
                    .lines()
                    .count(),
                expected_lines
            );
        }
        assert!(fs::read_to_string(directory.join("gpu-diagnostics.jsonl"))?.is_empty());
        drop(report);
        fs::remove_dir_all(directory)?;

        for (expected_state, error, cancelled) in [
            ("failed", Some("synthetic incomplete scan"), false),
            ("cancelled", None, true),
        ] {
            let mut partial = BenchmarkReport::new(json!({"synthetic_writer_test":true}))?;
            let directory = partial.directory().to_owned();
            partial.record_run("partial", json!({"input_seeds":2,"completed_seeds":1,"complete":false,"cancelled":cancelled,"threshold":600,"seconds":0.125,"seconds_are_synthetic":true}), &[(10000,650)])?;
            partial.finish(error, cancelled)?;
            let saved = read_json(&directory.join("report.json"))?;
            assert_eq!(saved["state"], expected_state);
            assert_eq!(saved["runs"][0]["complete"], false);
            assert_eq!(saved["runs"][0]["completed_seeds"], 1);
            drop(partial);
            fs::remove_dir_all(directory)?;
        }

        let directory = export_debug_log(json!({"synthetic_writer_test":true,"map_size":192}))?;
        let debug = read_json(&directory.join("debug-report.json"))?;
        assert_eq!(debug["kind"], "debug_snapshot");
        assert_eq!(debug["gpu_feature_enabled"], false);
        assert_eq!(debug["compile_build_info"], build_info);
        assert_eq!(debug["resolved_wgpu_version"], "unknown");
        assert_eq!(debug["recent_event_count"], 0);
        assert_eq!(debug["recent_events"], json!([]));
        assert_eq!(debug["ui_context"]["map_size"], 192);
        assert!(fs::read_to_string(directory.join("gpu-diagnostics.jsonl"))?.is_empty());
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}
