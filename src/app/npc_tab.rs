use crate::app::task::BackgroundTask;
use crate::npc::{
    generator::{extract_all_sect_npcs_with_settings, NpcGenerationSettings},
    types::{NpcData, PredictionStatus},
    GameData, SectData,
};
use eframe::egui;
use std::sync::{atomic::Ordering, Arc};

fn season_label(season: i32) -> &'static str {
    match season {
        1 => "春季",
        2 => "夏季",
        3 => "秋季",
        4 => "冬季",
        _ => "无季节",
    }
}

pub struct NpcScanOutcome {
    seed: i32,
    settings: NpcGenerationSettings,
    source: String,
    results: Vec<SectData>,
}

#[derive(Default)]
pub struct NpcTabState {
    pub target_seed: i32,
    pub result_seed: Option<i32>,
    pub generation_settings: NpcGenerationSettings,
    pub result_settings: Option<NpcGenerationSettings>,
    pub result_source: String,
    pub status_msg: String,
    pub settings_path: String,
    pub data_loaded: bool,
    pub load_failed: bool,
    pub sect_results: Vec<SectData>,
    pub game_data: Arc<GameData>,
    pub load_task: BackgroundTask<(Arc<GameData>, String)>,
    pub search_task: BackgroundTask<NpcScanOutcome>,
}

impl NpcTabState {
    pub fn is_busy(&self) -> bool {
        self.load_task.is_running || self.search_task.is_running
    }

    pub fn auto_load(&mut self) {
        if self.data_loaded || self.is_busy() || self.load_failed {
            return;
        }
        let result = self.load_task.start(1, |progress, _, _| {
            let data = GameData::load_embedded()?;
            data.validate()?;
            progress.store(1, Ordering::Relaxed);
            Ok((Arc::new(data), "内嵌数据".into()))
        });
        self.status_msg = result.map_or_else(|error| error, |()| "正在加载嵌入数据...".into());
    }

    pub fn update(&mut self) {
        self.auto_load();
        self.update_load();
        self.update_search();
    }

    fn update_load(&mut self) {
        let Some(result) = self.load_task.poll() else {
            return;
        };
        match result {
            Ok((data, source)) => {
                let count = data.jh_npcs_global.len();
                self.game_data = data;
                self.settings_path = source;
                self.data_loaded = true;
                self.load_failed = false;
                self.sect_results.clear();
                self.result_seed = None;
                self.result_settings = None;
                self.result_source.clear();
                self.status_msg = format!("数据加载成功，江湖 NPC 模板: {count} 个");
            }
            Err(error) => {
                self.status_msg = format!("加载失败: {error}");
                self.load_failed = true;
            }
        }
    }

    fn update_search(&mut self) {
        let Some(result) = self.search_task.poll() else {
            return;
        };
        match result {
            Ok(outcome) => self.accept_results(outcome),
            Err(error) => self.status_msg = format!("推演失败: {error}"),
        }
    }

    fn accept_results(&mut self, outcome: NpcScanOutcome) {
        let count: usize = outcome.results.iter().map(|sect| sect.npcs.len()).sum();
        let needs_state = outcome
            .results
            .iter()
            .flat_map(|sect| &sect.npcs)
            .filter(|npc| npc.prediction_status == PredictionStatus::RequiresWorldState)
            .count();
        self.result_seed = Some(outcome.seed);
        self.result_settings = Some(outcome.settings);
        self.result_source = outcome.source;
        self.sect_results = outcome.results;
        self.status_msg = format!(
            "Seed {} 推演完成，共 {count} 个 NPC；{needs_state} 个含需存档状态确定的字段",
            outcome.seed
        );
    }

    fn load_directory(&mut self) {
        let Some(path) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let source = path.display().to_string();
        self.load_failed = false;
        self.status_msg = format!("正在加载游戏数据: {source}");
        let result = self.load_task.start(1, move |progress, _, _| {
            let data = GameData::try_load_from_dir(&path)?;
            progress.store(1, Ordering::Relaxed);
            Ok((Arc::new(data), source))
        });
        if let Err(error) = result {
            self.status_msg = error;
        }
    }

    pub fn start_search(&mut self) {
        if self.is_busy() {
            return;
        }
        if !self.data_loaded {
            self.status_msg = "⚠️ 数据未加载，请等待或选择游戏目录".into();
            return;
        }
        let settings = match self.generation_settings.validate() {
            Ok(settings) => settings,
            Err(error) => {
                self.status_msg = error;
                return;
            }
        };
        let seed = self.target_seed;
        let data = Arc::clone(&self.game_data);
        let source = self.settings_path.clone();
        let result = self.search_task.start(1, move |progress, _, _| {
            data.validate()?;
            let results = extract_all_sect_npcs_with_settings(seed, &data, settings);
            if results.is_empty() {
                return Err("NPC 推演未生成任何门派".into());
            }
            progress.store(1, Ordering::Relaxed);
            Ok(NpcScanOutcome {
                seed,
                settings,
                source,
                results,
            })
        });
        self.status_msg = result.map_or_else(|error| error, |()| "⚙ 正在推演...".into());
    }

    pub fn render(&mut self, ui: &mut egui::Ui) {
        self.render_config(ui);
        self.render_controls(ui);
        self.render_results(ui);
    }

    fn render_config(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("npc_cfg_grid")
            .num_columns(2)
            .spacing([40.0, 15.0])
            .show(ui, |ui| {
                ui.label("游戏数据:");
                self.render_load_state(ui);
                ui.end_row();
                ui.label("更新数据 (可选):");
                self.render_data_source(ui);
                ui.end_row();
                ui.label("生成季节:");
                self.render_season(ui);
                ui.end_row();
                ui.label("战斗参数:");
                self.render_fight_settings(ui);
                ui.end_row();
                ui.label("世界 Seed:");
                ui.add(egui::DragValue::new(&mut self.target_seed).speed(1));
                ui.end_row();
            });
        ui.add_space(20.0);
    }

    fn render_load_state(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.data_loaded {
                ui.label(egui::RichText::new("✅ 已加载").color(egui::Color32::GREEN));
            } else if self.load_task.is_running {
                ui.spinner();
                ui.label("加载中...");
            } else if self.load_failed {
                ui.label(egui::RichText::new("❌ 加载失败").color(egui::Color32::RED));
            } else {
                ui.label(egui::RichText::new("❌ 未加载").color(egui::Color32::RED));
            }
        });
    }

    fn render_data_source(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!self.is_busy(), |ui| {
                if ui.button("从游戏目录加载").clicked() {
                    self.load_directory();
                }
            });
            if !self.settings_path.is_empty() {
                ui.label(&self.settings_path);
            }
        });
    }

    fn render_season(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::from_id_source("npc-season")
            .selected_text(season_label(self.generation_settings.season))
            .show_ui(ui, |ui| {
                for season in 0..=4 {
                    ui.selectable_value(
                        &mut self.generation_settings.season,
                        season,
                        season_label(season),
                    );
                }
            });
    }

    fn render_fight_settings(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.generation_settings.fight_difficult_coefficient)
                    .speed(0.1)
                    .clamp_range(0.1..=10.0)
                    .prefix("难度系数 "),
            );
            ui.checkbox(&mut self.generation_settings.fight_mode, "战斗模式");
        });
    }

    fn render_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!self.is_busy(), |ui| {
                if ui.button("推演").clicked() {
                    self.start_search();
                }
            });
            if self.is_busy() {
                ui.spinner();
            }
        });
        self.render_status(ui);
        if self.load_failed && !self.is_busy() && ui.button("重新加载嵌入数据").clicked() {
            self.load_failed = false;
            self.data_loaded = false;
            self.auto_load();
        }
        ui.separator();
    }

    fn render_status(&self, ui: &mut egui::Ui) {
        if self.status_msg.is_empty() {
            return;
        }
        let color = if self.status_msg.contains("失败") {
            egui::Color32::RED
        } else if self.status_msg.contains("⚠") {
            egui::Color32::YELLOW
        } else {
            egui::Color32::GREEN
        };
        ui.label(egui::RichText::new(&self.status_msg).color(color));
    }

    fn render_results(&self, ui: &mut egui::Ui) {
        ui.heading("姓名 / 符箓 / 行囊掉落物");
        if let Some(seed) = self.result_seed {
            ui.label(format!("以下结果对应 Seed {seed}"));
        }
        if !self.result_source.is_empty() {
            ui.label(format!("结果数据来源：{}", self.result_source));
        }
        if let Some(settings) = self.result_settings {
            ui.label(format!(
                "生成参数：{}，难度系数 {}，战斗模式 {}",
                season_label(settings.season),
                settings.fight_difficult_coefficient,
                settings.fight_mode
            ));
        }
        ui.add_space(10.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for sect in &self.sect_results {
                    render_sect(ui, sect);
                }
            });
    }
}

fn render_sect(ui: &mut egui::Ui, sect: &SectData) {
    egui::CollapsingHeader::new(
        egui::RichText::new(format!("{} ({}人)", sect.sect_name, sect.npcs.len()))
            .strong()
            .size(16.0),
    )
    .default_open(false)
    .show(ui, |ui| {
        for (index, npc) in sect.npcs.iter().enumerate() {
            egui::CollapsingHeader::new(format!("#{:02}: {}", index + 1, npc.name))
                .id_source((npc.school_id, index))
                .default_open(false)
                .show(ui, |ui| render_npc(ui, npc));
            ui.add_space(2.0);
        }
    });
}

fn render_npc(ui: &mut egui::Ui, npc: &NpcData) {
    for note in &npc.notes {
        ui.label(egui::RichText::new(note).color(egui::Color32::YELLOW));
    }
    ui.horizontal(|ui| {
        ui.group(|ui| render_talismans(ui, npc));
        ui.group(|ui| render_inventory(ui, npc));
    });
}

fn empty_items_label(npc: &NpcData, complete: &'static str) -> &'static str {
    match npc.prediction_status {
        PredictionStatus::RequiresWorldState => "需存档状态才能确定",
        PredictionStatus::Complete => complete,
    }
}

fn render_talismans(ui: &mut egui::Ui, npc: &NpcData) {
    ui.set_min_width(220.0);
    ui.label(egui::RichText::new("符箓").color(egui::Color32::LIGHT_BLUE));
    ui.separator();
    if npc.talismans.is_empty() {
        ui.label(empty_items_label(npc, "无"));
        return;
    }
    for item in &npc.talismans {
        ui.label(format!("✨ {} ({:.2})", item.name, item.quality));
    }
}

fn render_inventory(ui: &mut egui::Ui, npc: &NpcData) {
    ui.set_max_width((ui.available_width() - 30.0).max(100.0));
    ui.label(egui::RichText::new("行囊").color(egui::Color32::GOLD));
    ui.separator();
    ui.vertical(|ui| {
        if npc.inventory.is_empty() {
            ui.label(empty_items_label(npc, "空"));
            return;
        }
        for item in &npc.inventory {
            let quality = if item.quality >= 0.0 {
                format!(" ({:.2})", item.quality)
            } else {
                String::new()
            };
            ui.label(format!("{}{quality}", item.name));
        }
    });
}
