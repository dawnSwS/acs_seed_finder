use crate::rng::DotNetRandom;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

// 🌟 1:1 C# Dictionary 顺位模拟器
// 通过拦截插入去重，并将 values 严格以插入时间顺序存储，我们就能完美还原出乱数指针指向的真实对象！
struct CsDict<K, V> {
    keys: Vec<K>,
    pub values: Vec<V>,
    indices: HashMap<K, usize>,
}

impl<K: Eq + std::hash::Hash + Clone, V> CsDict<K, V> {
    fn new() -> Self {
        Self { keys: Vec::new(), values: Vec::new(), indices: HashMap::new() }
    }
    fn insert(&mut self, key: K, val: V) {
        if let Some(&idx) = self.indices.get(&key) {
            self.values[idx] = val; // 更新值，但不改变它在这个世界的位序
        } else {
            self.indices.insert(key.clone(), self.keys.len());
            self.keys.push(key);
            self.values.push(val);
        }
    }
}

pub struct GameData {
    pub clothes: Vec<String>,
    pub pants: Vec<String>,
    pub weapons: Vec<String>,
    pub stuffs: Vec<String>,
    pub spells: Vec<String>,
    pub loaded: bool,
}

impl GameData {
    pub fn new() -> Self {
        Self { clothes: Vec::new(), pants: Vec::new(), weapons: Vec::new(), stuffs: Vec::new(), spells: Vec::new(), loaded: false }
    }

    pub fn load_from_dir(base_path: &Path) -> Self {
        let mut item_dict = CsDict::new();
        let mut spell_dict = CsDict::new();
        let mut files = Vec::new();

        fn collect_xmls(dir: &Path, files: &mut Vec<PathBuf>) {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        collect_xmls(&path, files);
                    } else if path.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase()) == Some("xml".to_string()) {
                        files.push(path);
                    }
                }
            }
        }

        collect_xmls(&base_path.join("ThingDef"), &mut files);
        collect_xmls(&base_path.join("SpellDef"), &mut files);
        
        // C# API: Directory.GetFiles 返回的文件序列是基于文件系统字母字典序排序的
        files.sort();

        fn extract_blocks<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
            let mut blocks = Vec::new();
            let open = format!("<{}", tag);
            let close = format!("</{}>", tag);
            let mut current = xml;
            while let Some(start) = current.find(&open) {
                let after_open = &current[start..];
                if let Some(end) = after_open.find(&close) {
                    blocks.push(&after_open[..end + close.len()]);
                    current = &after_open[end + close.len()..];
                } else { break; }
            }
            blocks
        }

        fn get_node_text(block: &str, tag: &str) -> Option<String> {
            let open = format!("<{}>", tag);
            let close = format!("</{}>", tag);
            if let Some(start) = block.find(&open) {
                let rest = &block[start + open.len()..];
                if let Some(end) = rest.find(&close) {
                    return Some(rest[..end].trim().to_string());
                }
            }
            None
        }

        fn get_name(block: &str) -> String {
            if let Some(start) = block.find("Name=\"") {
                let rest = &block[start + 6..];
                if let Some(end) = rest.find("\"") { return rest[..end].to_string(); }
            }
            if let Some(start) = block.find("<defName>") {
                let rest = &block[start + 9..];
                if let Some(end) = rest.find("</defName>") { return rest[..end].trim().to_string(); }
            }
            String::new()
        }

        // 轻量极速截取 XML 节点
        for path in files {
            if let Ok(content) = fs::read_to_string(&path) {
                for block in extract_blocks(&content, "ThingDef") {
                    let name = get_name(block);
                    if name.is_empty() { continue; }
                    
                    let thing_name = get_node_text(block, "ThingName").unwrap_or_else(|| name.clone());
                    let item_type = get_node_text(block, "ItemType").unwrap_or_default();
                    let equip_type = get_node_text(block, "EquipType").unwrap_or_default();
                    
                    let mut is_stuff = false;
                    let is_stuff_str = get_node_text(block, "IsStuff").unwrap_or_default();
                    if is_stuff_str.to_lowercase() == "true" || is_stuff_str == "1" { is_stuff = true; }
                    if item_type == "Material" || item_type == "Stuff" { is_stuff = true; }
                    if block.contains("<StuffDef>") { is_stuff = true; }
                    
                    item_dict.insert(name, (thing_name, item_type, equip_type, is_stuff));
                }

                for block in extract_blocks(&content, "SpellDef") {
                    let name = get_name(block);
                    if name.is_empty() { continue; }
                    
                    let spell_name = get_node_text(block, "SpellName")
                        .or_else(|| get_node_text(block, "ThingName"))
                        .unwrap_or_else(|| name.clone());
                    spell_dict.insert(name, spell_name);
                }
            }
        }

        let mut data = Self::new();
        
        for (thing_name, item_type, equip_type, is_stuff) in item_dict.values {
            if equip_type == "Clothes" { data.clothes.push(thing_name.clone()); }
            if equip_type == "Pants" || equip_type == "Trousers" { data.pants.push(thing_name.clone()); }
            if equip_type == "Weapon" || item_type == "Fabao" { data.weapons.push(thing_name.clone()); }
            if is_stuff { data.stuffs.push(thing_name.clone()); }
        }
        
        data.spells = spell_dict.values;
        if !data.clothes.is_empty() { data.loaded = true; }
        
        data
    }
}

#[derive(Debug, Clone)]
pub struct AbstractLoot {
    pub category: String,
    pub item_name: String,
    pub stuff_name: String,
    pub quality: f32,
}

#[derive(Debug, Clone)]
pub struct NpcInventory {
    pub seed: i32,
    pub loots: Vec<AbstractLoot>,
    pub wealth: i32,
    pub has_jackpot: bool,
    pub best_fu_quality: f32,
}

pub fn exhaust_sect_elder_inventory(rng: &mut DotNetRandom, glevel: i32, is_vip: bool, data: &GameData) -> NpcInventory {
    let mut inventory = NpcInventory { seed: 0, loots: Vec::new(), wealth: 0, has_jackpot: false, best_fu_quality: 0.0 };

    let clothes_count = std::cmp::max(1, if data.loaded { data.clothes.len() as i32 } else { 45 });
    let pants_count   = std::cmp::max(1, if data.loaded { data.pants.len() as i32 } else { 45 });
    let weapon_count  = std::cmp::max(1, if data.loaded { data.weapons.len() as i32 } else { 45 });
    let spells_count  = std::cmp::max(1, if data.loaded { data.spells.len() as i32 } else { 50 });
    let stuff_cate_count = 10;
    let stuff_count   = std::cmp::max(1, if data.loaded { data.stuffs.len() as i32 } else { 80 });

    for part in ["衣服 (上衣)", "裤子 (下装)", "本命法宝"] {
        let pool_count = match part {
            "衣服 (上衣)" => clothes_count,
            "裤子 (下装)" => pants_count,
            "本命法宝" => weapon_count,
            _ => 1,
        };
        // 严格抽取，即使物品表只有 1 项，也要无条件吞噬一次环境游标！
        let item_idx = rng.next_range_strict(0, pool_count);
        let _cate_idx = rng.next_range_strict(0, stuff_cate_count);
        let stuff_idx = rng.next_range_strict(0, stuff_count);
        let q = rng.next_float(0.1, 0.5);
        
        let item_name = if data.loaded {
            match part {
                "衣服 (上衣)" => data.clothes.get(item_idx as usize).cloned().unwrap_or_default(),
                "裤子 (下装)" => data.pants.get(item_idx as usize).cloned().unwrap_or_default(),
                "本命法宝" => data.weapons.get(item_idx as usize).cloned().unwrap_or_default(),
                _ => String::new(),
            }
        } else { format!("{}_#{}", part, item_idx) };

        let stuff_name = if data.loaded {
            data.stuffs.get(stuff_idx as usize).cloned().unwrap_or_else(|| format!("材质_#{}", stuff_idx))
        } else { format!("材质_#{}", stuff_idx) };
        
        inventory.loots.push(AbstractLoot { category: part.to_string(), item_name, stuff_name, quality: q });
    }

    // 修仙者特有的随机判定
    if glevel > 0 {
        rng.next_range_strict(0, 12); 

        if glevel >= 6 {
            let mut gold = rng.next_range_strict(1000, 20000);
            let loop_chance = if is_vip { 0.8 } else { 0.4 };
            while rng.random_rate(loop_chance) { 
                gold += rng.next_range_strict(1000, 20000); 
            }
            inventory.wealth = gold;
        }

        if is_vip || rng.random_rate(0.3) { rng.box_muller_trap(); }

        let fu_chance = if is_vip { 0.6 } else { 0.2 };
        if rng.random_rate(fu_chance) {
            let spell_idx = rng.next_range_strict(0, spells_count); 
            let fu_quality = rng.next_float(0.1, 0.7); 
            
            // 天道雷达锁定绝世目标
            if fu_quality > 0.690 { inventory.has_jackpot = true; }
            inventory.best_fu_quality = fu_quality;

            let spell_name = if data.loaded {
                data.spells.get(spell_idx as usize).cloned().unwrap_or_else(|| format!("神符_#{}", spell_idx))
            } else { format!("神符_#{}", spell_idx) };

            inventory.loots.push(AbstractLoot {
                category: "神符 (Talisman)".to_string(), item_name: spell_name, stuff_name: "天道造化".to_string(), quality: fu_quality,
            });
        }
    }
    inventory
}