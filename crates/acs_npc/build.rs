use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir).join("game_data.bin.gz");

    // Step 1: 查找 Settings 目录
    let settings_dir = find_settings_dir();

    if let Some(settings_path) = settings_dir {
        eprintln!("📦 发现 Settings 目录: {}", settings_path.display());

        // 查找预打包的数据文件（在 Settings 同级目录）
        let packed_path = settings_path.parent().unwrap().join("game_data.bin.gz");
        if packed_path.exists() {
            eprintln!("📋 使用预打包数据: {}", packed_path.display());
            fs::copy(&packed_path, &out_path).unwrap();
            return;
        } else {
            eprintln!("⚠ 未找到预打包数据（在 Settings 目录旁）");
        }
    } else {
        eprintln!("⚠ 未找到 Settings 目录");
    }

    // Step 2: 回退 - 使用 crate 源码目录中已有的 game_data.bin.gz
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fallback_path = crate_dir.join("game_data.bin.gz");
    if fallback_path.exists() {
        eprintln!(
            "📋 使用 crate 目录中的预打包数据: {}",
            fallback_path.display()
        );
        fs::copy(&fallback_path, &out_path).unwrap();
        return;
    }

    // Step 3: 最后手段 - 写入空占位文件
    eprintln!("⚠ 未找到任何预打包数据，使用空占位文件");
    write_placeholder(&out_path);
}

fn find_settings_dir() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("../../../../Settings"),
        PathBuf::from("../../../Settings"),
        PathBuf::from("../../Settings"),
        PathBuf::from("../../../acs_work/Settings"),
    ];
    for p in &candidates {
        if p.exists() {
            return Some(p.clone());
        }
    }
    None
}

fn write_placeholder(path: &Path) {
    // 写入一个最小的 gzip 文件（解压后为空）
    let placeholder = &[
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x03, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    fs::write(path, placeholder).unwrap();
}
