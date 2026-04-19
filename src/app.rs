use eframe::egui;
use std::sync::{atomic::{AtomicUsize, Ordering}, mpsc, Arc};
use std::thread;
use std::path::Path;
use crate::{rng::*, scanner::*, gpu_scanner::*, hetero_scanner::*, sect_npc_scanner::*};

#[derive(PartialEq, Clone, Copy)] pub enum ComputeMode { CpuRayon, AmdGpuArchitecture, HeterogeneousPipeline }
#[derive(PartialEq, Clone, Copy)] pub enum AppTab { MapScanner, NpcScanner }

pub struct SeedFinderApp {
    app_tab: AppTab,
    map_size: i32, seed_start: i32, seed_end: i32, threshold: usize, limit_top_50: bool, string_seed: String,
    compute_mode: ComputeMode, is_searching: bool, progress: Arc<AtomicUsize>, total_tasks: usize,
    results: Vec<(i32, usize, String)>, rx: Option<mpsc::Receiver<Vec<(i32, usize, String)>>>, status_msg: String,
    is_benchmarking: bool, benchmark_results: Vec<(ComputeMode, f64)>, rx_bench: Option<mpsc::Receiver<Vec<(ComputeMode, f64)>>>,

    settings_path: String,
    npc_results: Vec<NpcInventory>,
    game_data: Arc<GameData>,
    npc_target_glevel: i32,
    npc_offset: usize,
    npc_vip: bool,
    rx_npc: Option<mpsc::Receiver<Vec<NpcInventory>>>,
}

impl Default for SeedFinderApp {
    fn default() -> Self {
        Self {
            app_tab: AppTab::MapScanner,
            map_size: 192, seed_start: 0, seed_end: 100000000, threshold: 5950, limit_top_50: true, string_seed: String::new(),
            compute_mode: ComputeMode::HeterogeneousPipeline, is_searching: false, progress: Arc::new(AtomicUsize::new(0)),
            total_tasks: 0, results: Vec::new(), rx: None, status_msg: String::new(),
            is_benchmarking: false, benchmark_results: Vec::new(), rx_bench: None,
            
            settings_path: String::new(),
            npc_results: Vec::new(),
            game_data: Arc::new(GameData::new()),
            npc_target_glevel: 9,
            npc_offset: 300,
            npc_vip: true,
            rx_npc: None,
        }
    }
}

impl eframe::App for SeedFinderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(res) => { self.results = res; self.is_searching = false; self.rx = None; }
                Err(mpsc::TryRecvError::Disconnected) => { self.is_searching = false; self.rx = None; self.status_msg = "⚠️ 扫描崩溃!".into(); }
                _ => {}
            }
        }
        if let Some(rx) = &self.rx_bench {
            match rx.try_recv() {
                Ok(res) => { self.benchmark_results = res; self.is_benchmarking = false; self.status_msg = "✅ 测试完成!".into(); self.rx_bench = None; }
                Err(mpsc::TryRecvError::Disconnected) => { self.is_benchmarking = false; self.rx_bench = None; self.status_msg = "⚠️ 测试崩溃!".into(); }
                _ => {}
            }
        }
        if let Some(rx) = &self.rx_npc {
            match rx.try_recv() {
                Ok(res) => { self.npc_results = res; self.is_searching = false; self.rx_npc = None; self.status_msg = "✅ 天道大能扫描完成!".into(); }
                Err(mpsc::TryRecvError::Disconnected) => { self.is_searching = false; self.rx_npc = None; self.status_msg = "⚠️ 扫描崩溃!".into(); }
                _ => {}
            }
        }
        
        if self.is_searching || self.is_benchmarking { ctx.request_repaint(); }

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.app_tab, AppTab::MapScanner, "🌍 地图灵土雷达");
                ui.selectable_value(&mut self.app_tab, AppTab::NpcScanner, "🔮 门派大能透视 (神符预测)");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.app_tab {
                AppTab::MapScanner => self.render_map_scanner(ui),
                AppTab::NpcScanner => self.render_npc_scanner(ui),
            }
        });
    }
}

impl SeedFinderApp {
    fn render_map_scanner(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("cfg").num_columns(2).spacing([40.0, 15.0]).show(ui, |ui| {
            ui.label("地图尺寸:"); ui.horizontal(|u| { u.radio_value(&mut self.map_size, 96, "小型"); u.radio_value(&mut self.map_size, 128, "中型"); u.radio_value(&mut self.map_size, 192, "大型"); }); ui.end_row();
            ui.label("计算模式:"); ui.horizontal(|u| { u.radio_value(&mut self.compute_mode, ComputeMode::CpuRayon, "💻 CPU"); u.radio_value(&mut self.compute_mode, ComputeMode::AmdGpuArchitecture, "🚀 GPU"); u.radio_value(&mut self.compute_mode, ComputeMode::HeterogeneousPipeline, "⚡ 混合计算"); }); ui.end_row();
            ui.label("输入文本:"); ui.horizontal(|u| { u.text_edit_singleline(&mut self.string_seed); if u.button("转为数字种子").clicked() && !self.string_seed.is_empty() { self.seed_start = string_hash(&self.string_seed); self.seed_end = self.seed_start; } }); ui.end_row();
            ui.label("种子区间:"); ui.horizontal(|u| { u.add(egui::DragValue::new(&mut self.seed_start).prefix("起: ")); u.label("至"); u.add(egui::DragValue::new(&mut self.seed_end).prefix("止: ")); }); ui.end_row();
            ui.label("灵土阈值:"); ui.horizontal(|u| { u.add(egui::DragValue::new(&mut self.threshold).speed(10).suffix(" 格")); u.add_space(20.0); u.checkbox(&mut self.limit_top_50, "仅输出 Top 50"); }); ui.end_row();
        });
        ui.add_space(20.0);

        if self.is_searching && self.app_tab == AppTab::MapScanner {
            ui.horizontal(|ui| { ui.spinner(); ui.label("扫描中..."); });
            let curr = self.progress.load(Ordering::Relaxed);
            ui.add(egui::ProgressBar::new(if self.total_tasks > 0 { (curr as f32 / self.total_tasks as f32).clamp(0.0, 1.0) } else { 0.0 }).show_percentage().text(format!("{}/{}", curr, self.total_tasks)));
        } else {
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!self.is_searching, |ui| {
                    if ui.button("▶ 开始扫描").clicked() && !self.is_benchmarking { self.start_search(); self.status_msg.clear(); }
                    ui.add_space(10.0);
                    if ui.button("📂 导入种子列表").clicked() { self.import_seeds(); }
                });
            });
            if !self.status_msg.is_empty() && self.app_tab == AppTab::MapScanner { ui.label(egui::RichText::new(&self.status_msg).color(egui::Color32::YELLOW)); }
        }
        ui.separator();

        ui.horizontal(|ui| {
            ui.heading("📊 基准测试");
            if self.is_benchmarking { ui.spinner(); } 
            else if ui.button("🚀 运行 2W 种子测试").clicked() && !self.is_searching { self.run_bench(); }
        });
        if !self.benchmark_results.is_empty() {
            egui::Grid::new("bench").striped(true).spacing([40.0, 10.0]).show(ui, |ui| {
                ui.strong("模式"); ui.strong("耗时(s)"); ui.strong("吞吐量(Seeds/s)"); ui.strong("倍率"); ui.end_row();
                let base_t = self.benchmark_results.iter().find(|(m, _)| *m == ComputeMode::CpuRayon).map(|(_, t)| *t).unwrap_or(1.0);
                for (mode, time) in &self.benchmark_results {
                    let t = if *time > 0.0 { *time } else { 0.0001 };
                    ui.label(match mode { ComputeMode::CpuRayon => "CPU", ComputeMode::AmdGpuArchitecture => "GPU", _ => "混合" });
                    ui.label(format!("{:.2} s", time)); ui.label(format!("{:.0}", 20_000.0 / t));
                    ui.label(egui::RichText::new(format!("{:.2}x", base_t / t)).color(if base_t / t > 1.5 { egui::Color32::GREEN } else { egui::Color32::WHITE })); ui.end_row();
                }
            });
        }
        ui.separator();

        ui.horizontal(|ui| {
            ui.heading(format!("🏆 扫描结果 (共 {} 个)", self.results.len()));
            if !self.results.is_empty() {
                ui.add_space(20.0);
                if ui.button("📋 复制全部文本").clicked() { ui.output_mut(|o| o.copied_text = self.results.iter().map(|(_, _, txt)| txt.clone()).collect::<Vec<_>>().join("\n")); }
                ui.add_space(10.0);
                if ui.button("🔢 复制全部数字").clicked() { ui.output_mut(|o| o.copied_text = self.results.iter().map(|(s, _, _)| s.to_string()).collect::<Vec<_>>().join("\n")); }
            }
        });
        
        ui.add_space(10.0);
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            if !self.results.is_empty() {
                egui::Grid::new("res").num_columns(4).striped(true).spacing([30.0, 10.0]).show(ui, |ui| {
                    ui.strong("排名"); ui.strong("文本种子"); ui.strong("灵土数量"); ui.strong("数字种子"); ui.end_row();
                    for (i, (s, c, col)) in self.results.iter().enumerate() {
                        ui.label(format!("#{}", i + 1)); if ui.button(col).clicked() { ui.output_mut(|o| o.copied_text = col.clone()); }
                        ui.label(format!("✨ {}", c)); if ui.button(s.to_string()).clicked() { ui.output_mut(|o| o.copied_text = s.to_string()); } ui.end_row();
                    }
                });
            }
        });
    }

    fn render_npc_scanner(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("npc_cfg").num_columns(2).spacing([40.0, 15.0]).show(ui, |ui| {
            ui.label("载入 Settings 目录:");
            ui.horizontal(|u| {
                if u.button("📂 选择").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.settings_path = path.display().to_string();
                        self.game_data = Arc::new(GameData::load_from_dir(Path::new(&self.settings_path)));
                        self.status_msg = format!("✅ 成功解析天道字典! 衣物:{} 材质:{} 符箓:{}", self.game_data.clothes.len(), self.game_data.stuffs.len(), self.game_data.spells.len());
                    }
                }
                u.label(if self.settings_path.is_empty() { "待选择 (若不选将使用抽象刻度代为输出)" } else { &self.settings_path });
            });
            ui.end_row();

            ui.label("修仙界种子区间:"); ui.horizontal(|u| { u.add(egui::DragValue::new(&mut self.seed_start).prefix("起: ")); u.label("至"); u.add(egui::DragValue::new(&mut self.seed_end).prefix("止: ")); }); ui.end_row();
            ui.label("游标环境垫刀 (Offset):"); ui.add(egui::DragValue::new(&mut self.npc_offset).speed(1)); ui.end_row();
            ui.label("目标大能:"); ui.horizontal(|u| { u.add(egui::DragValue::new(&mut self.npc_target_glevel).prefix("境界: ")); u.checkbox(&mut self.npc_vip, "掌门级(VIP, 将大幅触发财产吸收机制)"); }); ui.end_row();
        });
        ui.add_space(20.0);

        if self.is_searching && self.app_tab == AppTab::NpcScanner {
            ui.horizontal(|ui| { ui.spinner(); ui.label("天道透视雷达运行中..."); });
            let curr = self.progress.load(Ordering::Relaxed);
            ui.add(egui::ProgressBar::new(if self.total_tasks > 0 { (curr as f32 / self.total_tasks as f32).clamp(0.0, 1.0) } else { 0.0 }).show_percentage().text(format!("{}/{}", curr, self.total_tasks)));
        } else {
            ui.horizontal(|ui| {
                if ui.button("▶ 穷举绝世大能").clicked() { self.start_npc_search(); self.status_msg.clear(); }
            });
            if !self.status_msg.is_empty() && self.app_tab == AppTab::NpcScanner { ui.label(egui::RichText::new(&self.status_msg).color(egui::Color32::GREEN)); }
        }
        ui.separator();

        ui.horizontal(|ui| { ui.heading(format!("🏆 锁定大能 (共 {} 个)", self.npc_results.len())); });
        ui.add_space(10.0);
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            if !self.npc_results.is_empty() {
                for (i, inv) in self.npc_results.iter().enumerate() {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.strong(format!("#{} | 所在世界种子: {}", i + 1, inv.seed));
                            if let Some(col) = find_chinese_collision(inv.seed) {
                                ui.label(format!("({})", col));
                            }
                            if ui.button("📋 复制种子").clicked() { ui.output_mut(|o| o.copied_text = inv.seed.to_string()); }
                        });
                        ui.horizontal(|ui| {
                            ui.label(format!("💰 携带灵石: {}", inv.wealth));
                            ui.label(egui::RichText::new(format!("✨ 最高神韵极值: {:.4} (理论封顶0.7)", inv.best_fu_quality)).color(if inv.best_fu_quality > 0.69 { egui::Color32::GOLD } else { egui::Color32::WHITE }));
                        });
                        
                        egui::Grid::new(format!("loots_{}", inv.seed)).num_columns(4).striped(true).show(ui, |ui| {
                            ui.label("部位/类型"); ui.label("物资名称"); ui.label("材质"); ui.label("品质/神韵"); ui.end_row();
                            for loot in &inv.loots {
                                ui.label(&loot.category); ui.label(&loot.item_name); ui.label(&loot.stuff_name);
                                let q_text = if loot.quality > 0.69 { egui::RichText::new(format!("{:.4} ✨(绝品)", loot.quality)).color(egui::Color32::GOLD) } else { egui::RichText::new(format!("{:.4}", loot.quality)) };
                                ui.label(q_text); ui.end_row();
                            }
                        });
                    });
                    ui.add_space(5.0);
                }
            }
        });
    }

    fn start_search(&mut self) {
        if self.seed_start > self.seed_end { std::mem::swap(&mut self.seed_start, &mut self.seed_end); }
        self.is_searching = true; self.results.clear(); self.progress.store(0, Ordering::Relaxed);
        let (s, e, map, th, top50, prog, mode) = (self.seed_start, self.seed_end, self.map_size, self.threshold, self.limit_top_50, self.progress.clone(), self.compute_mode);
        self.total_tasks = ((e as i64) - (s as i64) + 1).max(0) as usize;
        let (tx, rx) = mpsc::channel(); self.rx = Some(rx);

        thread::spawn(move || {
            let mut res = match mode {
                ComputeMode::CpuRayon => scan_seeds(s, e, map, th, prog),
                ComputeMode::AmdGpuArchitecture => scan_seeds_amd_gpu(s, e, map, th, prog),
                ComputeMode::HeterogeneousPipeline => scan_seeds_heterogeneous(s, e, map, th, prog),
            };
            if top50 { res.truncate(50); }
            let _ = tx.send(res.into_iter().map(|(sd, ct)| (sd, ct, find_chinese_collision(sd).unwrap_or_else(|| "无解".into()))).collect());
        });
    }

    fn start_npc_search(&mut self) {
        if self.seed_start > self.seed_end { std::mem::swap(&mut self.seed_start, &mut self.seed_end); }
        self.is_searching = true; self.npc_results.clear(); self.progress.store(0, Ordering::Relaxed);
        let (s, e, offset, glevel, vip, prog, data) = (self.seed_start, self.seed_end, self.npc_offset, self.npc_target_glevel, self.npc_vip, self.progress.clone(), self.game_data.clone());
        self.total_tasks = ((e as i64) - (s as i64) + 1).max(0) as usize;
        let (tx, rx) = mpsc::channel(); self.rx_npc = Some(rx);

        thread::spawn(move || {
            use rayon::prelude::*;
            let mut res: Vec<_> = (s..=e).into_par_iter().filter_map(|seed| {
                let mut rng = DotNetRandom::new(seed);
                rng.advance(offset);
                let mut inv = exhaust_sect_elder_inventory(&mut rng, glevel, vip, &data);
                inv.seed = seed;
                prog.fetch_add(1, Ordering::Relaxed);
                if inv.has_jackpot {
                    Some(inv)
                } else {
                    None
                }
            }).collect();
            // 根据最高神韵排序
            res.sort_by(|a, b| b.best_fu_quality.partial_cmp(&a.best_fu_quality).unwrap());
            if res.len() > 100 { res.truncate(100); }
            let _ = tx.send(res);
        });
    }

    fn run_bench(&mut self) {
        self.is_benchmarking = true; self.benchmark_results.clear(); self.status_msg = "⏱ 正在测试 (样本: 2W)...".into();
        let (tx, rx) = mpsc::channel(); self.rx_bench = Some(rx);
        let prog = Arc::new(AtomicUsize::new(0));

        thread::spawn(move || {
            let mut bench = Vec::new();
            for &(mode, f) in &[(ComputeMode::CpuRayon, scan_seeds as fn(i32,i32,i32,usize,Arc<AtomicUsize>) -> _), 
                                (ComputeMode::AmdGpuArchitecture, scan_seeds_amd_gpu), 
                                (ComputeMode::HeterogeneousPipeline, scan_seeds_heterogeneous)] {
                prog.store(0, Ordering::Relaxed);
                let t = std::time::Instant::now(); f(10000, 29999, 192, 600, prog.clone());
                bench.push((mode, t.elapsed().as_secs_f64()));
            }
            let _ = tx.send(bench);
        });
    }

    fn import_seeds(&mut self) {
        if let Some(path) = rfd::FileDialog::new().add_filter("文本", &["txt", "csv"]).pick_file() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let seeds: Vec<i32> = content.lines().filter_map(|l| l.split_whitespace().last()?.parse().ok()).collect();
                if seeds.is_empty() { self.status_msg = "⚠️ 未找到有效种子".into(); return; }
                self.is_searching = true; self.results.clear(); self.progress.store(0, Ordering::Relaxed); self.total_tasks = seeds.len();
                let (map, th, top50, prog) = (self.map_size, self.threshold, self.limit_top_50, self.progress.clone());
                let (tx, rx) = mpsc::channel(); self.rx = Some(rx);

                thread::spawn(move || {
                    let mut res = scan_seed_list(seeds, map, th, prog);
                    if top50 { res.truncate(50); }
                    let _ = tx.send(res.into_iter().map(|(sd, ct)| (sd, ct, find_chinese_collision(sd).unwrap_or_else(|| "无解".into()))).collect());
                });
                self.status_msg = format!("✅ 导入 {} 个种子", self.total_tasks);
            }
        }
    }
}