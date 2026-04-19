#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app; mod gpu_scanner; mod hetero_scanner; mod map_maker; mod rng; mod scanner; mod terrain; mod sect_npc_scanner;

fn main() -> eframe::Result<()> {
    eframe::run_native("了不起的修仙模拟器 - 极品种子扫描仪",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default().with_inner_size([800.0, 650.0]).with_min_inner_size([600.0, 500.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            let mut fonts = eframe::egui::FontDefinitions::default();
            for path in ["C:\\Windows\\Fonts\\msyh.ttc", "C:\\Windows\\Fonts\\msyh.ttf", "C:\\Windows\\Fonts\\simhei.ttf"] {
                if let Ok(data) = std::fs::read(path) {
                    fonts.font_data.insert("cjk".into(), eframe::egui::FontData::from_owned(data));
                    fonts.families.get_mut(&eframe::egui::FontFamily::Proportional).unwrap().insert(0, "cjk".into());
                    fonts.families.get_mut(&eframe::egui::FontFamily::Monospace).unwrap().insert(0, "cjk".into());
                    cc.egui_ctx.set_fonts(fonts); break;
                }
            }
            Box::new(app::SeedFinderApp::default())
        })
    )
}