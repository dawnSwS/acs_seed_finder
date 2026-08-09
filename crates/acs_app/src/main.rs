#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
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
            let mut f = eframe::egui::FontDefinitions::default();
            if let Some(d) = [
                "C:\\Windows\\Fonts\\msyh.ttc",
                "C:\\Windows\\Fonts\\msyh.ttf",
                "C:\\Windows\\Fonts\\simhei.ttf",
            ]
            .iter()
            .find_map(|p| std::fs::read(p).ok())
            {
                f.font_data
                    .insert("cjk".into(), eframe::egui::FontData::from_owned(d));
                f.families
                    .values_mut()
                    .for_each(|fam| fam.insert(0, "cjk".into()));
                cc.egui_ctx.set_fonts(f);
            }
            Box::new(app::SeedFinderApp::default())
        }),
    )
}
