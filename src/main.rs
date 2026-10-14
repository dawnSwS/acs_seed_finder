#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![recursion_limit = "256"]

mod app;
mod npc;

use acs_seed_finder::{core, scanner};

fn font_definitions() -> eframe::egui::FontDefinitions {
    use eframe::egui::{FontData, FontDefinitions};
    let font = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyh.ttf",
        "C:\\Windows\\Fonts\\simhei.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/System/Library/Fonts/PingFang.ttc",
    ]
    .iter()
    .find_map(|path| std::fs::read(path).ok())
    .map_or_else(
        || FontData::from_static(include_bytes!("../assets/fonts/NotoSansCJKsc-Regular.otf")),
        FontData::from_owned,
    );
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("cjk".into(), font);
    for family in fonts.families.values_mut() {
        family.insert(0, "cjk".into());
    }
    fonts
}

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "了不起的修仙模拟器 - 造化玉牒",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([800., 650.])
                .with_min_inner_size([600., 500.]),
            ..Default::default()
        },
        Box::new(|cc| {
            cc.egui_ctx.set_fonts(font_definitions());
            Box::new(app::SeedFinderApp::default())
        }),
    )
}
