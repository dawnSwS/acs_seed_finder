#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app; mod gpu_scanner; mod hetero_scanner; mod map_maker; mod rng; mod scanner; mod terrain; mod sect_npc_scanner;

fn main() -> eframe::Result<()> {
    eframe::run_native("了不起的修仙模拟器 - 极品种子扫描仪", eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([800.0, 650.0]).with_min_inner_size([600.0, 500.0]),
        ..Default::default()
    }, Box::new(|cc| {
        let mut fonts = eframe::egui::FontDefinitions::default();
        if let Some(data) = ["C:\\Windows\\Fonts\\msyh.ttc", "C:\\Windows\\Fonts\\msyh.ttf", "C:\\Windows\\Fonts\\simhei.ttf"].iter().find_map(|p| std::fs::read(p).ok()) {
            fonts.font_data.insert("cjk".into(), eframe::egui::FontData::from_owned(data));
            fonts.families.values_mut().for_each(|f| f.insert(0, "cjk".into()));
            cc.egui_ctx.set_fonts(fonts);
        }
        Ok(Box::new(app::SeedFinderApp::default()))
    }))
}