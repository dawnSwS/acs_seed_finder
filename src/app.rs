use eframe::egui;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use crate::rng::{string_hash, find_chinese_collision};
use crate::scanner::{scan_seeds, scan_seed_list}; 
use crate::gpu_scanner::scan_seeds_amd_gpu;
use crate::hetero_scanner::scan_seeds_heterogeneous;

#[derive(PartialEq, Clone, Copy)]
pub enum ComputeMode {
    CpuRayon,
    AmdGpuArchitecture,
    HeterogeneousPipeline,
}

pub struct SeedFinderApp {
    map_size: i32,
    seed_start: i32,
    seed_end: i32,
    threshold: usize,
    limit_top_50: bool,
    string_seed: String,
    compute_mode: ComputeMode,
    is_searching: bool,
    progress: Arc<AtomicUsize>,
    total_tasks: usize,
    results: Vec<(i32, usize, String)>,
    rx: Option<mpsc::Receiver<Vec<(i32, usize, String)>>>,
    dump_status: String,
    
    is_benchmarking: bool,
    benchmark_status: String,
    benchmark_results: Vec<(ComputeMode, f64)>,
    rx_bench: Option<mpsc::Receiver<Vec<(ComputeMode, f64)>>>,
}

impl Default for SeedFinderApp {
    fn default() -> Self {
        Self {
            map_size: 192,
            seed_start: 0,
            seed_end: 100000000,
            threshold: 5950,
            limit_top_50: true,
            string_seed: String::new(),
            compute_mode: ComputeMode::HeterogeneousPipeline,
            is_searching: false,
            progress: Arc::new(AtomicUsize::new(0)),
            total_tasks: 0,
            results: Vec::new(),
            rx: None,
            dump_status: String::new(),
            
            is_benchmarking: false,
            benchmark_status: String::new(),
            benchmark_results: Vec::new(),
            rx_bench: None,
        }
    }
}

impl eframe::App for SeedFinderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(res) => {
                    self.results = res;
                    self.is_searching = false;
                    self.rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.is_searching = false;
                    self.rx = None;
                    self.dump_status = "⚠️ 扫描线程崩溃 (Panic)，请检查终端输出！".to_string();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        if let Some(rx_bench) = &self.rx_bench {
            match rx_bench.try_recv() {
                Ok(res) => {
                    self.benchmark_results = res;
                    self.is_benchmarking = false;
                    self.benchmark_status = "✅ 基准测试完成！".to_string();
                    self.rx_bench = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.is_benchmarking = false;
                    self.benchmark_status = "⚠️ 基准测试线程崩溃！".to_string();
                    self.rx_bench = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        if self.is_searching || self.is_benchmarking { ctx.request_repaint(); }

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::Grid::new("config_grid")
                .num_columns(2)
                .spacing([40.0, 15.0])
                .show(ui, |ui| {
                    ui.label("地图尺寸:");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.map_size, 96, "小型 (96)");
                        ui.radio_value(&mut self.map_size, 128, "中型 (128)");
                        ui.radio_value(&mut self.map_size, 192, "大型 (192)");
                    });
                    ui.end_row();

                    ui.label("计算模式:");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.compute_mode, ComputeMode::CpuRayon, "💻 CPU 并发 (Rayon)");
                        ui.radio_value(&mut self.compute_mode, ComputeMode::AmdGpuArchitecture, "🚀 GPU 计算 (WGPU)");
                        ui.radio_value(&mut self.compute_mode, ComputeMode::HeterogeneousPipeline, "⚡ 混合计算 (GPU 初筛 + CPU 精算)");
                    });
                    ui.end_row();

                    ui.label("输入文本种子:");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.string_seed);
                        if ui.button("转换为数字种子").clicked() {
                            if !self.string_seed.is_empty() {
                                let hash = string_hash(&self.string_seed);
                                self.seed_start = hash;
                                self.seed_end = hash;
                            }
                        }
                    });
                    ui.end_row();

                    ui.label("数字种子区间:");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut self.seed_start).prefix("起: "));
                        ui.label("至");
                        ui.add(egui::DragValue::new(&mut self.seed_end).prefix("止: "));
                    });
                    ui.end_row();

                    ui.label("极品灵土阈值:");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut self.threshold).speed(10).suffix(" 格"));
                        ui.add_space(20.0);
                        ui.checkbox(&mut self.limit_top_50, "限制仅输出 Top 50"); 
                    });
                    ui.end_row();
            });

            ui.add_space(20.0);

            if self.is_searching {
                ui.horizontal(|ui| {
                    ui.spinner();
                    match self.compute_mode {
                        ComputeMode::CpuRayon => ui.label("💻 CPU 扫描中，请稍候..."),
                        ComputeMode::AmdGpuArchitecture => ui.label("🚀 GPU 扫描中，请稍候..."),
                        ComputeMode::HeterogeneousPipeline => ui.label("⚡ 混合扫描中 (GPU 初筛 -> CPU 精算)..."),
                    }
                });
                
                let current = self.progress.load(Ordering::Relaxed);
                let fraction = if self.total_tasks > 0 {
                    (current as f32 / self.total_tasks as f32).clamp(0.0, 1.0)
                } else { 0.0 };
                ui.add(egui::ProgressBar::new(fraction).show_percentage().text(format!("{}/{}", current, self.total_tasks)));
            } else {
                ui.horizontal(|ui| {
                    if ui.button("▶ 开始扫描").clicked() {
                        if !self.is_benchmarking {
                            self.start_search();
                            self.dump_status.clear();
                        }
                    }
                    ui.add_space(10.0);
                    
                    if ui.button("🐛 生成日志").on_hover_text("导出 Rust_RNG_Dump.csv").clicked() {
                        self.dump_rng_log();
                    }
                    
                    ui.add_space(10.0);
                    if ui.button("📂 导入种子列表并扫描").on_hover_text("选择 txt 格式的种子列表，自动提取并进行扫描过滤").clicked() {
                        self.import_and_search_seeds();
                    }
                });
                
                if !self.dump_status.is_empty() {
                    ui.label(egui::RichText::new(&self.dump_status).color(egui::Color32::YELLOW));
                }
            }

            ui.add_space(15.0);
            ui.separator();
            ui.heading("📊 性能基准测试 (Benchmark)");
            ui.horizontal(|ui| {
                if self.is_benchmarking {
                    ui.spinner();
                    ui.label(egui::RichText::new(&self.benchmark_status).color(egui::Color32::YELLOW));
                } else {
                    if ui.button("🚀 运行 2 万种子性能测试").clicked() {
                        if !self.is_searching {
                            self.run_benchmark();
                        }
                    }
                    if !self.benchmark_status.is_empty() {
                        ui.label(egui::RichText::new(&self.benchmark_status).color(egui::Color32::LIGHT_GREEN));
                    }
                }
            });

            if !self.benchmark_results.is_empty() {
                ui.add_space(10.0);
                egui::Grid::new("bench_grid").striped(true).spacing([40.0, 10.0]).show(ui, |ui| {
                    ui.strong("计算架构");
                    ui.strong("耗时 (秒)");
                    ui.strong("吞吐量 (Seeds/s)");
                    ui.strong("性能倍率 (vs CPU)");
                    ui.end_row();

                    let baseline_time = self.benchmark_results.iter()
                        .find(|(m, _)| *m == ComputeMode::CpuRayon)
                        .map(|(_, t)| *t)
                        .unwrap_or(1.0);

                    for (mode, time) in &self.benchmark_results {
                        let mode_str = match mode {
                            ComputeMode::CpuRayon => "💻 CPU 并发 (Rayon)",
                            ComputeMode::AmdGpuArchitecture => "🚀 GPU 计算 (WGPU)",
                            ComputeMode::HeterogeneousPipeline => "⚡ 混合模式 (CPU+GPU)",
                        };
                        
                        let safe_time = if *time > 0.0 { *time } else { 0.0001 };
                        let throughput = 20_000.0 / safe_time;
                        let multiplier = baseline_time / safe_time;

                        ui.label(mode_str);
                        ui.label(format!("{:.2} s", time));
                        ui.label(format!("{:.0}", throughput));
                        
                        let color = if multiplier > 1.5 { egui::Color32::GREEN } 
                                    else if multiplier < 0.8 { egui::Color32::RED } 
                                    else { egui::Color32::WHITE };
                        ui.label(egui::RichText::new(format!("{:.2}x", multiplier)).color(color));
                        ui.end_row();
                    }
                });
            }

            ui.add_space(15.0);
            ui.separator();
            
            ui.horizontal(|ui| {
                ui.heading(format!("🏆 扫描结果 (共 {} 个)", self.results.len()));
                
                if !self.results.is_empty() {
                    ui.add_space(20.0);
                    if ui.button(format!("📋 复制全部 {} 个文本种子", self.results.len())).clicked() {
                        let all_collisions = self.results.iter()
                            .map(|(_, _, collision)| collision.clone())
                            .collect::<Vec<String>>()
                            .join("\n"); 
                        ui.output_mut(|o| o.copied_text = all_collisions);
                    }
                    ui.add_space(10.0);
                    if ui.button(format!("🔢 复制全部 {} 个数字种子", self.results.len())).clicked() {
                        let all_seeds = self.results.iter()
                            .map(|(seed, _, _)| seed.to_string())
                            .collect::<Vec<String>>()
                            .join("\n"); 
                        ui.output_mut(|o| o.copied_text = all_seeds);
                    }
                }
            });

            ui.add_space(10.0);
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                if !self.results.is_empty() {
                    egui::Grid::new("results_grid")
                        .num_columns(4).striped(true).spacing([30.0, 10.0])
                        .show(ui, |ui| {
                            ui.strong("排名"); ui.strong("文本种子 (一键复制使用)"); ui.strong("灵土数量"); ui.strong("数字种子"); ui.end_row();
                            for (i, (seed, count, collision)) in self.results.iter().enumerate() {
                                ui.label(format!("#{}", i + 1));
                                if ui.button(format!("📋 {}", collision)).clicked() { ui.output_mut(|o| o.copied_text = collision.to_string()); }
                                ui.label(format!("✨ {}", count));
                                if ui.button(format!("🔢 {}", seed)).clicked() { ui.output_mut(|o| o.copied_text = seed.to_string()); }
                                ui.end_row();
                            }
                        });
                }
            });
        });
    }
}

impl SeedFinderApp {
    fn start_search(&mut self) {
        if self.seed_start > self.seed_end { std::mem::swap(&mut self.seed_start, &mut self.seed_end); }
        self.is_searching = true;
        self.results.clear();
        self.dump_status.clear();
        self.progress.store(0, Ordering::Relaxed);

        let start = self.seed_start; let end = self.seed_end;
        let map_size = self.map_size; let threshold = self.threshold;
        let limit_top_50 = self.limit_top_50;
        let progress = self.progress.clone();
        
        let c_mode = self.compute_mode;

        let diff = (end as i64) - (start as i64) + 1;
        self.total_tasks = diff.max(0) as usize;

        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);

        thread::spawn(move || {
            let mut local_results = match c_mode {
                ComputeMode::CpuRayon => scan_seeds(start, end, map_size, threshold, progress),
                ComputeMode::AmdGpuArchitecture => scan_seeds_amd_gpu(start, end, map_size, threshold, progress),
                ComputeMode::HeterogeneousPipeline => scan_seeds_heterogeneous(start, end, map_size, threshold, progress),
            };
            
            if limit_top_50 {
                local_results.truncate(50);
            }

            let final_results: Vec<(i32, usize, String)> = local_results.into_iter()
                .map(|(seed, count)| {
                    let collision_str = find_chinese_collision(seed).unwrap_or_else(|| "暂无解".to_string());
                    (seed, count, collision_str)
                }).collect();

            let _ = tx.send(final_results);
        });
    }

    fn run_benchmark(&mut self) {
        self.is_benchmarking = true;
        self.benchmark_results.clear();
        self.benchmark_status = "⏱ 正在进行基准测试 (样本量: 20,000)... 这可能需要几分钟，请勿操作。".to_string();

        let (tx, rx) = mpsc::channel();
        self.rx_bench = Some(rx);

        let start_seed = 10000;
        let end_seed = 29999;
        let map_size = 192;
        let threshold = 600;
        let progress = Arc::new(AtomicUsize::new(0));

        thread::spawn(move || {
            let mut results = Vec::new();

            let p1 = progress.clone();
            p1.store(0, Ordering::Relaxed);
            let t0 = std::time::Instant::now();
            crate::scanner::scan_seeds(start_seed, end_seed, map_size, threshold, p1);
            results.push((ComputeMode::CpuRayon, t0.elapsed().as_secs_f64()));

            let p2 = progress.clone();
            p2.store(0, Ordering::Relaxed);
            let t1 = std::time::Instant::now();
            crate::gpu_scanner::scan_seeds_amd_gpu(start_seed, end_seed, map_size, threshold, p2);
            results.push((ComputeMode::AmdGpuArchitecture, t1.elapsed().as_secs_f64()));

            let p3 = progress.clone();
            p3.store(0, Ordering::Relaxed);
            let t2 = std::time::Instant::now();
            crate::hetero_scanner::scan_seeds_heterogeneous(start_seed, end_seed, map_size, threshold, p3);
            results.push((ComputeMode::HeterogeneousPipeline, t2.elapsed().as_secs_f64()));

            let _ = tx.send(results);
        });
    }

    fn import_and_search_seeds(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("文本文件", &["txt", "csv"])
            .pick_file() 
        {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                let mut seeds = Vec::new();
                for line in contents.lines() {
                    let line = line.trim();
                    if line.is_empty() { continue; }
                    
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(last_part) = parts.last() {
                        if let Ok(seed) = last_part.parse::<i32>() {
                            seeds.push(seed);
                        }
                    }
                }
                
                if seeds.is_empty() {
                    self.dump_status = "⚠️ 文件中没有找到有效的数字种子".to_string();
                    return;
                }

                self.is_searching = true;
                self.results.clear();
                self.dump_status.clear();
                self.progress.store(0, Ordering::Relaxed);
                self.total_tasks = seeds.len();

                let map_size = self.map_size;
                let threshold = self.threshold;
                let limit_top_50 = self.limit_top_50;
                let progress = self.progress.clone();
                
                let (tx, rx) = mpsc::channel();
                self.rx = Some(rx);

                thread::spawn(move || {
                    let mut local_results = scan_seed_list(seeds, map_size, threshold, progress);
                    
                    if limit_top_50 {
                        local_results.truncate(50);
                    }

                    let final_results: Vec<(i32, usize, String)> = local_results.into_iter()
                        .map(|(seed, count)| {
                            let collision_str = crate::rng::find_chinese_collision(seed).unwrap_or_else(|| "暂无解".to_string());
                            (seed, count, collision_str)
                        }).collect();

                    let _ = tx.send(final_results);
                });

                self.dump_status = format!("✅ 成功导入 {} 个种子并进行扫描", self.total_tasks);
            } else {
                self.dump_status = "⚠️ 无法读取选择的文件，请检查文件权限或编码".to_string();
            }
        }
    }

    fn dump_rng_log(&mut self) {
        let mut maker = crate::map_maker::MapMaker::new(self.seed_start, self.map_size, self.map_size);
        maker.rand.enable_logging("Rust_RNG_Dump.csv");
        maker.make_map();
        
        let count = maker.grid.iter().filter(|&&t| t == crate::terrain::Terrain::LingSoil).count();
        self.results.clear();
        let collision_str = crate::rng::find_chinese_collision(self.seed_start).unwrap_or_else(|| "暂无解".to_string());
        self.results.push((self.seed_start, count, collision_str));
        
        self.dump_status = format!("✅ 成功将数字种子 {} 的日志输出至工作目录 Rust_RNG_Dump.csv", self.seed_start);
    }
}