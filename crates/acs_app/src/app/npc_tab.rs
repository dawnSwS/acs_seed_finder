use crate::app::task::BackgroundTask;
use acs_npc::{extract_all_sect_npcs, GameData, SectData};
use eframe::egui;
use std::sync::Arc;

#[derive(Default)]
pub struct NpcTabState {
    pub target_seed: i32,
    pub status_msg: String,
    pub settings_path: String,
    pub data_loaded: bool,
    pub load_failed: bool,
    pub sect_results: Vec<SectData>,
    pub game_data: Arc<GameData>,
    pub load_task: BackgroundTask<Result<Arc<GameData>, String>>,
    pub search_task: BackgroundTask<Vec<SectData>>,
}

impl NpcTabState {
    pub fn is_busy(&self) -> bool {
        self.load_task.is_running || self.search_task.is_running
    }

    /// 启动时自动加载嵌入数据
    pub fn auto_load(&mut self) {
        if self.data_loaded || self.load_task.is_running || self.load_failed {
            return;
        }
        self.status_msg = "⚙ 正在加载嵌入数据...".into();
        self.load_task
            .start(1, |_, _| match GameData::load_embedded() {
                Ok(data) => Ok(Arc::new(data)),
                Err(e) => Err(e.to_string()),
            });
    }

    pub fn update(&mut self, _ctx: &egui::Context) {
        // 自动触发加载
        if !self.data_loaded
            && !self.load_task.is_running
            && !self.load_failed
            && self.game_data.jh_npcs_global.is_empty()
        {
            self.auto_load();
        }

        if let Some(r) = self.load_task.poll() {
            match r {
                Ok(Ok(Ok(data))) => {
                    let count = data.jh_npcs_global.len();
                    self.game_data = data;
                    self.data_loaded = count > 0;
                    self.load_failed = false;
                    if count > 0 {
                        self.status_msg = format!("✅ 嵌入数据加载成功！江湖NPC: {} 个", count);
                    } else {
                        self.status_msg = "⚠ 嵌入数据为空，请选择游戏目录重新打包".into();
                        self.load_failed = true;
                    }
                }
                Ok(Ok(Err(e))) => {
                    self.status_msg = format!("💥 加载失败: {}", e);
                    self.load_failed = true;
                }
                Ok(Err(e)) => {
                    self.status_msg = format!("💥 加载线程异常: {}", e);
                    self.load_failed = true;
                }
                Err(_) => {
                    self.status_msg = "💥 加载线程通道异常!".into();
                    self.load_failed = true;
                }
            }
        }
        if let Some(r) = self.search_task.poll() {
            match r {
                Ok(Ok(res)) => {
                    self.sect_results = res;
                    self.status_msg = "✅ 推演完毕。怀璧明鉴已就绪。".into();
                }
                Ok(Err(e)) => {
                    self.status_msg = format!("💥 推演崩溃: {}", e);
                }
                Err(_) => {
                    self.status_msg = "💥 推演线程异常!".into();
                }
            }
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui) {
        // 配置区
        egui::Grid::new("npc_cfg_grid")
            .num_columns(2)
            .spacing([40.0, 15.0])
            .show(ui, |ui| {
                ui.label("游戏数据:");
                ui.horizontal(|u| {
                    if self.data_loaded {
                        u.label(egui::RichText::new("✅ 已加载").color(egui::Color32::GREEN));
                    } else if self.load_task.is_running {
                        u.spinner();
                        u.label("加载中...");
                    } else if self.load_failed {
                        u.label(egui::RichText::new("❌ 加载失败").color(egui::Color32::RED));
                    } else {
                        u.label(egui::RichText::new("❌ 未加载").color(egui::Color32::RED));
                    }
                });
                ui.end_row();

                ui.label("更新数据 (可选):");
                ui.horizontal(|u| {
                    u.add_enabled_ui(!self.is_busy(), |u| {
                        if u.button("📁 选择游戏目录重新打包").clicked() {
                            if let Some(p) = rfd::FileDialog::new().pick_folder() {
                                let path_str = p.display().to_string();
                                self.settings_path = path_str.clone();
                                self.load_failed = false;
                                self.status_msg = "⚙ 正在从游戏目录加载数据...".into();
                                self.load_task.start(1, move |_, _| {
                                    Ok(Arc::new(GameData::load_from_dir(std::path::Path::new(
                                        &path_str,
                                    ))))
                                });
                            }
                        }
                    });
                    if !self.settings_path.is_empty() {
                        u.label(&self.settings_path);
                    }
                });
                ui.end_row();

                ui.label("世界 Seed:");
                ui.horizontal(|u| {
                    u.add(egui::DragValue::new(&mut self.target_seed).speed(1));
                });
                ui.end_row();
            });
        ui.add_space(20.0);

        // 推演按钮
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!self.is_busy(), |ui| {
                let btn = ui.button("🔮 推演");
                if btn.clicked() {
                    if self.data_loaded {
                        let seed = self.target_seed;
                        let data = self.game_data.clone();
                        self.status_msg = "⚙ 正在推演...".into();
                        self.search_task
                            .start(1, move |_, _| extract_all_sect_npcs(seed, &data));
                    } else {
                        self.status_msg = "⚠️ 数据未加载，请等待或选择游戏目录".into();
                    }
                }
            });
            if self.is_busy() {
                ui.spinner();
            }
        });

        // 状态消息
        if !self.status_msg.is_empty() {
            let color = if self.status_msg.contains("💥") {
                egui::Color32::RED
            } else if self.status_msg.contains("⚠") {
                egui::Color32::YELLOW
            } else {
                egui::Color32::GREEN
            };
            ui.label(egui::RichText::new(&self.status_msg).color(color));
        }
        ui.separator();

        // 结果显示
        ui.heading("📜 姓名 / 符箓 / 行囊掉落物");
        ui.add_space(10.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for sect in &self.sect_results {
                    egui::CollapsingHeader::new(
                        egui::RichText::new(format!(
                            "🏯 {} ({}人)",
                            sect.sect_name,
                            sect.npcs.len()
                        ))
                        .strong()
                        .size(16.0),
                    )
                    .default_open(false)
                    .show(ui, |ui| {
                        for (e_idx, npc) in sect.npcs.iter().enumerate() {
                            egui::CollapsingHeader::new(format!(
                                "🧙 #{:02}: {}",
                                e_idx + 1,
                                npc.name
                            ))
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.group(|ui| {
                                        ui.set_min_width(220.0);
                                        ui.label(
                                            egui::RichText::new("📜 符箓")
                                                .color(egui::Color32::LIGHT_BLUE),
                                        );
                                        ui.separator();
                                        if npc.talismans.is_empty() {
                                            ui.label("❓ 无");
                                        } else {
                                            for t in &npc.talismans {
                                                ui.label(format!(
                                                    "✨ {} ({:.2})",
                                                    t.name, t.quality
                                                ));
                                            }
                                        }
                                    });
                                    ui.group(|ui| {
                                        ui.set_max_width(ui.available_width() - 30.0);
                                        ui.label(
                                            egui::RichText::new("🎒 行囊")
                                                .color(egui::Color32::GOLD),
                                        );
                                        ui.separator();
                                        ui.vertical(|ui| {
                                            if npc.inventory.is_empty() {
                                                ui.label("❓ 空");
                                            } else {
                                                for item in &npc.inventory {
                                                    let q_str = if item.quality > 0.0 {
                                                        format!(" ({:.2})", item.quality)
                                                    } else {
                                                        String::new()
                                                    };
                                                    ui.label(format!("📦 {}{}", item.name, q_str));
                                                }
                                            }
                                        });
                                    });
                                });
                            });
                            ui.add_space(2.0);
                        }
                    });
                }
            });
    }
}
