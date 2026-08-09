use crate::app::task::BackgroundTask;
use acs_core::scanner_api::{ComputeMode, ScanConfig, SeedScanner};
use acs_core::utils::{find_chinese_collision, string_hash};
use acs_scanners::EngineScanner;
use eframe::egui;
use std::sync::atomic::Ordering;

pub struct MapTabState {
    pub map_size: i32,
    pub seed_start: i32,
    pub seed_end: i32,
    pub threshold: usize,
    pub limit_top_50: bool,
    pub string_seed: String,
    pub compute_mode: ComputeMode,
    pub status_msg: String,
    pub results: Vec<(i32, usize, String)>,
    pub benchmark_results: Vec<(ComputeMode, f64)>,
    pub search_task: BackgroundTask<Vec<(i32, usize, String)>>,
    pub bench_task: BackgroundTask<Vec<(ComputeMode, f64)>>,
}

impl Default for MapTabState {
    fn default() -> Self {
        Self {
            map_size: 192,
            seed_start: 0,
            seed_end: 100_000_000,
            threshold: 5950,
            limit_top_50: true,
            string_seed: "".into(),
            compute_mode: ComputeMode::default(),
            status_msg: "".into(),
            results: vec![],
            benchmark_results: vec![],
            search_task: BackgroundTask::default(),
            bench_task: BackgroundTask::default(),
        }
    }
}

impl MapTabState {
    pub fn is_busy(&self) -> bool {
        self.search_task.is_running || self.bench_task.is_running
    }

    pub fn update(&mut self, _ctx: &egui::Context) {
        if let Some(r) = self.search_task.poll() {
            match r {
                Ok(Ok(res)) => {
                    self.results = res;
                    self.status_msg.clear();
                }
                Ok(Err(e)) => {
                    self.status_msg = format!("💥 搜索崩溃: {}", e);
                }
                Err(e) => {
                    self.status_msg = format!("💥 搜索线程异常: {:?}", e);
                }
            }
        }
        if let Some(r) = self.bench_task.poll() {
            match r {
                Ok(Ok(res)) => {
                    self.benchmark_results = res;
                    self.status_msg = "✅ 测试完成!".into();
                }
                Ok(Err(e)) => {
                    self.status_msg = format!("💥 测试崩溃: {}", e);
                }
                Err(e) => {
                    self.status_msg = format!("💥 测试线程异常: {:?}", e);
                }
            }
        }
    }

    pub fn start_search(&mut self) {
        if self.seed_start > self.seed_end {
            std::mem::swap(&mut self.seed_start, &mut self.seed_end);
        }
        self.results.clear();
        let (s, e, map, th, top50, mode) = (
            self.seed_start,
            self.seed_end,
            self.map_size,
            self.threshold,
            self.limit_top_50,
            self.compute_mode,
        );
        self.search_task.start(
            e.saturating_sub(s).saturating_add(1) as usize,
            move |p, _| {
                let mut r = EngineScanner::new(mode).scan(
                    s,
                    e,
                    &ScanConfig {
                        map_size: map,
                        threshold: th,
                    },
                    p,
                );
                if top50 {
                    r.truncate(50);
                }
                r.into_iter()
                    .map(|(sd, ct)| {
                        (
                            sd,
                            ct,
                            find_chinese_collision(sd).unwrap_or_else(|| "无解".into()),
                        )
                    })
                    .collect()
            },
        );
    }

    pub fn run_bench(&mut self) {
        self.benchmark_results.clear();
        self.status_msg = "⚙ 正在测试 (样本: 2W)...".into();
        self.bench_task.start(0, move |prog, _| {
            [ComputeMode::CpuRayon, ComputeMode::HeterogeneousPipeline]
                .into_iter()
                .map(|m| {
                    prog.store(0, Ordering::Relaxed);
                    let t = std::time::Instant::now();
                    EngineScanner::new(m).scan(
                        10000,
                        29999,
                        &ScanConfig {
                            map_size: 192,
                            threshold: 600,
                        },
                        prog.clone(),
                    );
                    (m, t.elapsed().as_secs_f64())
                })
                .collect()
        });
    }

    pub fn import_seeds(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("文本", &["txt", "csv"])
            .pick_file()
        {
            self.results.clear();
            let (map, th, top50) = (self.map_size, self.threshold, self.limit_top_50);
            self.status_msg = "⚙ 正在后台安全读取文件并解析...".into();
            self.search_task.start(1, move |prog, total| {
                if let Ok(c) = std::fs::read_to_string(p) {
                    let seeds: Vec<i32> = c
                        .lines()
                        .filter_map(|l| l.split_whitespace().next_back()?.parse().ok())
                        .collect();
                    if seeds.is_empty() {
                        return vec![];
                    }
                    total.store(seeds.len(), Ordering::Relaxed);
                    let mut r = EngineScanner::new(ComputeMode::CpuRayon).scan_list(
                        seeds,
                        &ScanConfig {
                            map_size: map,
                            threshold: th,
                        },
                        prog,
                    );
                    if top50 {
                        r.truncate(50);
                    }
                    r.into_iter()
                        .map(|(sd, ct)| {
                            (
                                sd,
                                ct,
                                find_chinese_collision(sd).unwrap_or_else(|| "无解".into()),
                            )
                        })
                        .collect()
                } else {
                    vec![]
                }
            });
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui) {
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
                    [
                        (ComputeMode::CpuRayon, "🧠 CPU"),
                        (ComputeMode::HeterogeneousPipeline, "⚡ 混合计算"),
                    ]
                    .into_iter()
                    .for_each(|(v, t)| {
                        u.radio_value(&mut self.compute_mode, v, t);
                    });
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

        if self.search_task.is_running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("扫描中...");
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
                    if ui.button("🚀 开始扫描").clicked() {
                        self.start_search();
                        self.status_msg.clear();
                    }
                    ui.add_space(10.0);
                    if ui.button("📂 导入种子列表").clicked() {
                        self.import_seeds();
                    }
                })
            });
            if !self.status_msg.is_empty() {
                ui.label(egui::RichText::new(&self.status_msg).color(egui::Color32::YELLOW));
            }
        }
        ui.separator();

        ui.horizontal(|ui| {
            ui.heading("⚡ 基准测试");
            if self.bench_task.is_running {
                ui.spinner();
            } else if ui.button("🏃 运行 2W 测试").clicked() && !self.search_task.is_running {
                self.run_bench();
            }
        });
        if !self.benchmark_results.is_empty() {
            egui::Grid::new("bench")
                .striped(true)
                .spacing([40.0, 10.0])
                .show(ui, |ui| {
                    ["模式", "耗时(s)", "吞吐量(S/s)", "倍率"]
                        .iter()
                        .for_each(|&h| {
                            ui.strong(h);
                        });
                    ui.end_row();
                    let base_t = self
                        .benchmark_results
                        .iter()
                        .find(|(m, _)| *m == ComputeMode::CpuRayon)
                        .map_or(1.0, |&(_, t)| t);
                    for (m, t) in &self.benchmark_results {
                        let ts = t.max(0.0001);
                        ui.label(match m {
                            ComputeMode::CpuRayon => "CPU",
                            ComputeMode::HeterogeneousPipeline => "混合",
                        });
                        ui.label(format!("{t:.2} s"));
                        ui.label(format!("{:.0}", 20_000.0 / ts));
                        ui.label(egui::RichText::new(format!("{:.2}x", base_t / ts)).color(
                            if base_t / ts > 1.5 {
                                egui::Color32::GREEN
                            } else {
                                egui::Color32::WHITE
                            },
                        ));
                        ui.end_row();
                    }
                });
        }
        ui.separator();

        ui.horizontal(|ui| {
            ui.heading(format!("📋 扫描结果 (共 {} 个)", self.results.len()));
            if !self.results.is_empty() {
                ui.add_space(20.0);
                if ui.button("📋 复制全部文本").clicked() {
                    ui.output_mut(|o| {
                        o.copied_text = self
                            .results
                            .iter()
                            .map(|(_, _, t)| t.clone())
                            .collect::<Vec<_>>()
                            .join("\n")
                    });
                }
                ui.add_space(10.0);
                if ui.button("🔢 复制全部数字").clicked() {
                    ui.output_mut(|o| {
                        o.copied_text = self
                            .results
                            .iter()
                            .map(|(s, _, _)| s.to_string())
                            .collect::<Vec<_>>()
                            .join("\n")
                    });
                }
            }
        });
        ui.add_space(10.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if !self.results.is_empty() {
                    egui::Grid::new("res")
                        .num_columns(4)
                        .striped(true)
                        .spacing([30.0, 10.0])
                        .show(ui, |ui| {
                            ["排名", "文本种子", "灵土数量", "数字种子"]
                                .iter()
                                .for_each(|&h| {
                                    ui.strong(h);
                                });
                            ui.end_row();
                            for (i, (s, c, col)) in self.results.iter().enumerate() {
                                ui.label(format!("#{}", i + 1));
                                if ui.button(col).clicked() {
                                    ui.output_mut(|o| o.copied_text = col.clone());
                                }
                                ui.label(format!("✨ {c}"));
                                if ui.button(s.to_string()).clicked() {
                                    ui.output_mut(|o| o.copied_text = s.to_string());
                                }
                                ui.end_row();
                            }
                        });
                }
            });
    }
}
