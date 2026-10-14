use eframe::egui;

pub mod map_tab;
pub mod npc_tab;
pub mod task;

pub use {map_tab::MapTabState, npc_tab::NpcTabState};

#[derive(PartialEq, Clone, Copy, Default)]
pub enum AppTab {
    #[default]
    MapScanner,
    NpcScanner,
}

#[derive(Default)]
pub struct SeedFinderApp {
    pub selected: AppTab,
    pub map_tab: MapTabState,
    pub npc_tab: NpcTabState,
}

impl eframe::App for SeedFinderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.map_tab.update();
        self.npc_tab.update();
        if self.map_tab.is_busy() || self.npc_tab.is_busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.selected, AppTab::MapScanner, "灵土扫描");
                ui.selectable_value(&mut self.selected, AppTab::NpcScanner, "NPC 扫描");
            })
        });
        egui::CentralPanel::default().show(ctx, |ui| match self.selected {
            AppTab::MapScanner => self.map_tab.render(ui),
            AppTab::NpcScanner => self.npc_tab.render(ui),
        });
    }
}
