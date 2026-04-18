#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod gpu_scanner;
mod hetero_scanner;
mod map_maker;
mod rng;
mod scanner;
mod terrain;
use eframe::egui;
fn main() -> eframe::Result<()> {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([720.0, 600.0])
            .with_min_inner_size([500.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "了不起的修仙模拟器 - 极品种子扫描仪",
        options,
        Box::new(|cc| {
            setup_custom_fonts(&cc.egui_ctx);
            Box::new(app::SeedFinderApp::default())
        }),
    )
}
fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let font_paths = ["C:\\Windows\\Fonts\\msyh.ttc", "C:\\Windows\\Fonts\\msyh.ttf", "C:\\Windows\\Fonts\\simhei.ttf"];
    for path in font_paths {
        if let Ok(font_data) = std::fs::read(path) {
            fonts.font_data.insert("cjk".to_owned(), egui::FontData::from_owned(font_data));
            if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Proportional) { fam.insert(0, "cjk".to_owned()); }
            if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Monospace) { fam.insert(0, "cjk".to_owned()); }
            ctx.set_fonts(fonts);
            break;
        }
    }
}