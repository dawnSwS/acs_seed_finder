use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

use acs_npc::types::*;
use acs_npc::GameData;
use acs_npc::PackedGameData;
use flate2::write::GzEncoder;
use flate2::Compression;

const DATA_MAGIC: u32 = 0x5043_5341; // "ACSP"
const DATA_VERSION: u32 = 1;

#[derive(serde::Serialize)]
struct PackedDataFile {
    magic: u32,
    version: u32,
    data: PackedGameData,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let settings_path = if args.len() > 1 {
        Path::new(&args[1]).to_path_buf()
    } else {
        // 默认查找 ../Settings 或 ../../../Settings
        let candidates = [
            Path::new("../../Settings"),
            Path::new("../../../Settings"),
            Path::new("../../../acs_work/Settings"),
        ];
        let mut found = None;
        for p in &candidates {
            if p.exists() {
                found = Some(p.to_path_buf());
                break;
            }
        }
        found.expect("Settings directory not found. Usage: pack_data <settings_path>")
    };

    eprintln!("📦 开始打包游戏数据...");
    eprintln!("   Settings 路径: {}", settings_path.display());

    // 使用 acs_npc 的 GameData 加载所有数据
    let game_data = GameData::load_from_dir(&settings_path);

    eprintln!("✅ 数据加载完成:");
    eprintln!("   - 翻译条目: {}", game_data.translations.len());
    eprintln!("   - 姓名前缀: {}", game_data.names_prefix.len());
    eprintln!("   - 男性后缀: {}", game_data.names_m_suffix.len());
    eprintln!("   - 女性后缀: {}", game_data.names_f_suffix.len());
    eprintln!("   - 物品定义: {}", game_data.thing_defs.len());
    eprintln!("   - 法术列表: {}", game_data.spells_list.len());
    eprintln!("   - 修饰符: {}", game_data.modifier_defs.len());
    eprintln!("   - 江湖NPC: {}", game_data.jh_npcs_global.len());

    // 转换为可序列化的格式
    let packed = PackedGameData {
        translations: game_data.translations,
        names_prefix: game_data.names_prefix,
        names_m_suffix: game_data.names_m_suffix,
        names_f_suffix: game_data.names_f_suffix,
        fabao_prefix_lengths: game_data.fabao_prefix_lengths,
        fabao_mid_lengths: game_data.fabao_mid_lengths,
        modifier_defs: game_data.modifier_defs,
        spells_list: game_data.spells_list,
        thing_defs: game_data.thing_defs,
        map_lable2item: game_data.map_lable2item,
        map_lable2stuff: game_data.map_lable2stuff,
        jh_npcs_global: game_data.jh_npcs_global,
        name_to_index: game_data.name_to_index,
        school_to_names: game_data.school_to_names,
        location_to_template: game_data.location_to_template,
    };

    // 包装为带 magic header 的格式
    let data_file = PackedDataFile {
        magic: DATA_MAGIC,
        version: DATA_VERSION,
        data: packed,
    };

    // 序列化为 bincode
    eprintln!("🔄 序列化为 bincode...");
    let bincode_data = bincode::serialize(&data_file).expect("Failed to serialize");
    eprintln!(
        "   格式版本: v{}, Magic: 0x{:08X}",
        DATA_VERSION, DATA_MAGIC
    );

    // gzip 压缩
    eprintln!("🗜️  gzip 压缩中...");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&bincode_data).expect("Failed to write");
    let compressed = encoder.finish().expect("Failed to compress");

    // 输出到文件
    let output_path = if args.len() > 2 {
        Path::new(&args[2]).to_path_buf()
    } else {
        let out_dir = env::var("OUT_DIR").unwrap_or_else(|_| ".".to_string());
        Path::new(&out_dir).join("game_data.bin.gz")
    };

    fs::write(&output_path, &compressed).expect("Failed to write output file");

    let original_size = bincode_data.len();
    let compressed_size = compressed.len();
    let ratio = (compressed_size as f64 / original_size as f64 * 100.0) as u32;

    eprintln!("✅ 打包完成!");
    eprintln!("   输出文件: {}", output_path.display());
    eprintln!("   原始大小: {} KB", original_size / 1024);
    eprintln!("   压缩大小: {} KB", compressed_size / 1024);
    eprintln!("   压缩率: {}%", ratio);
}
