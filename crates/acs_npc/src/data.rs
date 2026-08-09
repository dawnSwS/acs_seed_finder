use super::{crypto::read_and_decrypt, types::*, xml::parse_xml};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

/// 嵌入的预处理游戏数据（gzip压缩的bincode二进制）
pub static EMBEDDED_DATA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/game_data.bin.gz"));

/// Magic number identifying valid packed data files: "ACSP" in little-endian
const DATA_MAGIC: u32 = 0x5043_5341; // "ACSP"
/// Current packed data format version — bump when schema changes
const DATA_VERSION: u32 = 1;

/// Wrapper struct that adds a magic header and version for validation
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PackedDataFile {
    pub magic: u32,
    pub version: u32,
    pub data: PackedGameData,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct PackedGameData {
    pub translations: HashMap<String, String>,
    pub names_prefix: Vec<String>,
    pub names_m_suffix: Vec<String>,
    pub names_f_suffix: Vec<String>,
    pub fabao_prefix_lengths: HashMap<i32, i32>,
    pub fabao_mid_lengths: HashMap<i32, i32>,
    pub modifier_defs: HashMap<String, ModifierDefData>,
    pub spells_list: Vec<SpellDefData>,
    pub thing_defs: HashMap<String, ThingDefData>,
    pub map_lable2item: HashMap<String, Vec<String>>,
    pub map_lable2stuff: HashMap<String, Vec<String>>,
    pub jh_npcs_global: Vec<JianghuNpcDef>,
    pub name_to_index: HashMap<String, usize>,
    pub school_to_names: HashMap<i32, Vec<String>>,
    pub location_to_template: HashMap<i32, String>,
}

#[derive(Default)]
pub struct GameData {
    pub jh_npcs_global: Vec<JianghuNpcDef>,
    pub name_to_index: HashMap<String, usize>,
    pub school_to_names: HashMap<i32, Vec<String>>,
    pub names_prefix: Vec<String>,
    pub names_m_suffix: Vec<String>,
    pub names_f_suffix: Vec<String>,
    pub translations: HashMap<String, String>,
    pub thing_defs: HashMap<String, ThingDefData>,
    pub map_lable2item: HashMap<String, Vec<String>>,
    pub map_lable2stuff: HashMap<String, Vec<String>>,
    pub modifier_defs: HashMap<String, ModifierDefData>,
    pub fabao_prefix_lengths: HashMap<i32, i32>,
    pub fabao_mid_lengths: HashMap<i32, i32>,
    pub spells_list: Vec<SpellDefData>,
    pub location_to_template: HashMap<i32, String>,
}

fn walk_dir(dir: &Path, files: &mut Vec<PathBuf>, ext: &str) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_dir(&path, files, ext);
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(ext))
            {
                files.push(path);
            }
        }
    }
}

impl GameData {
    pub fn load_from_dir(base_path: &Path) -> Self {
        let mut d = Self::default();
        let settings_dir = if let Some(name) = base_path.file_name() {
            if !name.to_string_lossy().eq_ignore_ascii_case("settings") {
                base_path.join("Settings")
            } else {
                base_path.to_path_buf()
            }
        } else {
            base_path.join("Settings")
        };
        d.load_translations(&settings_dir);
        d.load_names(&settings_dir);
        d.load_fabao_lengths(&settings_dir);
        d.load_modifiers(&settings_dir);
        d.load_spells(&settings_dir);
        d.load_things(&settings_dir);
        d.load_npcs(&settings_dir);
        d
    }
    /// 从嵌入的预处理数据加载（极快，无IO开销）
    pub fn load_embedded() -> Result<Self, String> {
        use std::io::Read;
        let mut decoder = flate2::read::GzDecoder::new(EMBEDDED_DATA);
        let mut buf = Vec::new();
        decoder
            .read_to_end(&mut buf)
            .map_err(|e| format!("解压失败: {}", e))?;
        if buf.is_empty() {
            return Err(
                "嵌入数据为空（占位文件），请在 Settings 目录旁运行 pack_data 重新打包".into(),
            );
        }
        let file: PackedDataFile = bincode::deserialize(&buf).map_err(|e| {
            format!(
                "反序列化失败: {} — 嵌入数据可能已损坏或版本不匹配，请运行 pack_data 重新打包",
                e
            )
        })?;
        if file.magic != DATA_MAGIC {
            return Err(format!(
                "嵌入数据校验失败: magic 期望 0x{:08X}，实际 0x{:08X}，文件可能已损坏",
                DATA_MAGIC, file.magic
            ));
        }
        if file.version != DATA_VERSION {
            return Err(format!(
                "嵌入数据版本不匹配: 期望 v{}，实际 v{}，请用最新版 pack_data 重新打包",
                DATA_VERSION, file.version
            ));
        }
        let packed = file.data;
        Ok(Self {
            jh_npcs_global: packed.jh_npcs_global,
            name_to_index: packed.name_to_index,
            school_to_names: packed.school_to_names,
            location_to_template: packed.location_to_template,
            names_prefix: packed.names_prefix,
            names_m_suffix: packed.names_m_suffix,
            names_f_suffix: packed.names_f_suffix,
            translations: packed.translations,
            thing_defs: packed.thing_defs,
            map_lable2item: packed.map_lable2item,
            map_lable2stuff: packed.map_lable2stuff,
            modifier_defs: packed.modifier_defs,
            fabao_prefix_lengths: packed.fabao_prefix_lengths,
            fabao_mid_lengths: packed.fabao_mid_lengths,
            spells_list: packed.spells_list,
        })
    }
    pub fn translate(&self, key: &str) -> String {
        self.translations
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }
    fn load_translations(&mut self, settings_dir: &Path) {
        let mut f_lang = vec![];
        walk_dir(&settings_dir.join("Language"), &mut f_lang, "txt");
        walk_dir(&settings_dir.join("Language"), &mut f_lang, "xml");
        for p in f_lang {
            if let Ok(content) = read_and_decrypt(&p) {
                if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt")) {
                    for chunk in content
                        .lines()
                        .map(|l| l.trim())
                        .filter(|l| !l.is_empty())
                        .collect::<Vec<_>>()
                        .chunks(2)
                    {
                        if chunk.len() == 2 && !chunk[0].is_empty() && !chunk[1].is_empty() {
                            self.translations
                                .insert(chunk[0].to_string(), chunk[1].trim().to_string());
                        }
                    }
                } else {
                    for root in parse_xml(&content) {
                        let mut dfs = vec![root];
                        while let Some(n) = dfs.pop() {
                            dfs.extend(n.children.clone());
                            if n.tag.eq_ignore_ascii_case("Text")
                                || n.tag.eq_ignore_ascii_case("li")
                                || n.tag.eq_ignore_ascii_case("XmlText")
                            {
                                if let Some(key) = n.get_val("Name") {
                                    let val = n
                                        .get_val("DisplayName")
                                        .or_else(|| n.get_val("ThingName"))
                                        .or_else(|| n.get_val("Desc"))
                                        .or_else(|| n.get_val("Text"))
                                        .unwrap_or_else(|| n.text.clone());
                                    if !key.is_empty() && !val.trim().is_empty() {
                                        self.translations.insert(key, val.trim().to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    fn load_names(&mut self, settings_dir: &Path) {
        let mut f_names = vec![];
        walk_dir(&settings_dir.join("Display/NpcName"), &mut f_names, "txt");
        f_names.sort_by(|a, b| {
            a.to_string_lossy()
                .to_lowercase()
                .cmp(&b.to_string_lossy().to_lowercase())
        });
        for p in f_names {
            let fname = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if fname.contains("school")
                || fname.contains("city")
                || fname.contains("place")
                || fname.contains("randomgong")
                || fname.contains("naught")
                || fname.contains("panda")
                || fname.contains("yaoguai")
                || fname.contains("animal")
                || fname.contains("monster")
                || fname.contains("tradename")
            {
                continue;
            }
            if let Ok(content) = read_and_decrypt(&p) {
                let words: Vec<String> = content
                    .split(|c: char| c == '\n' || c == '\r')
                    .filter_map(|line| {
                        let s = line
                            .split(|c: char| c == ',' || c == '\t')
                            .next()?
                            .trim_matches(|c| c == '\u{feff}' || c == ' ' || c == '\t' || c == '"')
                            .to_string();
                        if s.is_empty() || s.starts_with("//") {
                            None
                        } else {
                            Some(s)
                        }
                    })
                    .collect();
                let is_f_suffix = fname.contains("fsuffix")
                    || fname.ends_with("f.txt")
                    || fname.contains("suffixnamef")
                    || fname.contains("_f")
                    || fname.contains("female");
                let is_m_suffix = !is_f_suffix
                    && (fname.contains("msuffix")
                        || fname.ends_with("m.txt")
                        || fname.contains("suffixnamem")
                        || fname.contains("_m")
                        || fname.contains("male"));
                if fname.contains("prefix") {
                    self.names_prefix.extend(words);
                } else if is_m_suffix {
                    self.names_m_suffix.extend(words);
                } else if is_f_suffix {
                    self.names_f_suffix.extend(words);
                } else if fname.contains("suffix") {
                    self.names_m_suffix.extend(words.clone());
                    self.names_f_suffix.extend(words);
                }
            }
        }
    }
    fn load_fabao_lengths(&mut self, settings_dir: &Path) {
        if let Ok(content) = read_and_decrypt(&settings_dir.join("Display/FabaoName/Prefix.txt")) {
            for (i, line) in content.lines().enumerate() {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() > 1 {
                    self.fabao_prefix_lengths
                        .insert(i as i32 + 1, parts[1].split(',').count() as i32);
                }
            }
        }
        if let Ok(content) = read_and_decrypt(&settings_dir.join("Display/FabaoName/Mid.txt")) {
            for (i, line) in content.lines().enumerate() {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() > 1 {
                    self.fabao_mid_lengths
                        .insert(i as i32 + 1, parts[1].split(',').count() as i32);
                }
            }
        }
    }
    fn load_modifiers(&mut self, settings_dir: &Path) {
        let mut f_mod = vec![];
        walk_dir(&settings_dir.join("Modifiers"), &mut f_mod, "xml");
        for p in f_mod {
            if let Ok(content) = read_and_decrypt(&p) {
                for root in parse_xml(&content) {
                    let mut dfs = vec![root];
                    while let Some(n) = dfs.pop() {
                        dfs.extend(n.children.clone());
                        if n.tag.eq_ignore_ascii_case("Modifier")
                            || n.tag.eq_ignore_ascii_case("ModifierDef")
                            || n.tag.eq_ignore_ascii_case("li")
                        {
                            if let Some(name) = n.get_val("Name") {
                                let mut mdef = ModifierDefData {
                                    name: name.clone(),
                                    properties: Vec::new(),
                                };
                                if let Some(props) = n
                                    .children
                                    .iter()
                                    .find(|c| c.tag.eq_ignore_ascii_case("Properties"))
                                {
                                    for li in &props.children {
                                        if li.tag.eq_ignore_ascii_case("li") {
                                            let p_name = li.get_val("Name").unwrap_or_default();
                                            let add_v = li
                                                .get_val("AddV")
                                                .and_then(|s| s.parse().ok())
                                                .unwrap_or(0.0);
                                            let add_p = li
                                                .get_val("AddP")
                                                .and_then(|s| s.parse().ok())
                                                .unwrap_or(0.0);
                                            if !p_name.is_empty() {
                                                mdef.properties.push(ModifierPropertyData {
                                                    name: p_name,
                                                    add_v,
                                                    add_p,
                                                });
                                            }
                                        }
                                    }
                                }
                                self.modifier_defs.insert(name, mdef);
                            }
                        }
                    }
                }
            }
        }
    }
    fn load_spells(&mut self, settings_dir: &Path) {
        let mut f_spell = vec![];
        walk_dir(&settings_dir.join("Practice/Spell"), &mut f_spell, "xml");
        f_spell.sort_by(|a, b| {
            a.to_string_lossy()
                .to_lowercase()
                .cmp(&b.to_string_lossy().to_lowercase())
        });
        let (mut temp_spells, mut parsed_spells) = (HashMap::new(), Vec::new());
        for p in f_spell {
            if let Ok(content) = read_and_decrypt(&p) {
                for root in parse_xml(&content) {
                    let mut dfs = vec![root];
                    while let Some(n) = dfs.pop() {
                        dfs.extend(n.children.clone());
                        if n.tag.eq_ignore_ascii_case("SpellDef")
                            || n.tag.eq_ignore_ascii_case("li")
                            || n.tag.eq_ignore_ascii_case("Spell")
                        {
                            if let Some(name) = n.get_val("Name") {
                                if !name.is_empty() {
                                    temp_spells.insert(
                                        name.clone(),
                                        (
                                            n.get_val("Parent").unwrap_or_default(),
                                            n.get_val("Template").unwrap_or_default(),
                                        ),
                                    );
                                    if !parsed_spells.contains(&name) {
                                        parsed_spells.push(name.clone());
                                    }
                                    if let Some(disp) =
                                        n.get_val("DisplayName").or_else(|| n.get_val("ThingName"))
                                    {
                                        if !disp.trim().is_empty() {
                                            self.translations.insert(name, disp.trim().to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        for name in parsed_spells {
            if let Some((parent, template)) = temp_spells.get(&name) {
                let (mut curr_template, mut curr_parent, mut depth) =
                    (template.clone(), parent.clone(), 0);
                while curr_template.is_empty() && !curr_parent.is_empty() && depth < 10 {
                    if let Some((p_parent, p_template)) = temp_spells.get(&curr_parent) {
                        curr_template = p_template.clone();
                        curr_parent = p_parent.clone();
                    } else {
                        break;
                    }
                    depth += 1;
                }
                let has_template = !curr_template.is_empty();
                if let Some(existing) = self.spells_list.iter_mut().find(|s| s.name == name) {
                    existing.has_template = has_template;
                } else {
                    self.spells_list.push(SpellDefData {
                        name: name.clone(),
                        has_template,
                    });
                }
            }
        }
    }
    fn load_things(&mut self, settings_dir: &Path) {
        let mut f_thing = vec![];
        walk_dir(&settings_dir.join("ThingDef"), &mut f_thing, "xml");
        f_thing.sort_by(|a, b| {
            a.to_string_lossy()
                .to_lowercase()
                .cmp(&b.to_string_lossy().to_lowercase())
        });
        let (mut temp_thing_defs, mut parsed_things) = (HashMap::new(), Vec::new());
        for p in f_thing {
            if let Ok(content) = read_and_decrypt(&p) {
                for root in parse_xml(&content) {
                    let mut dfs = vec![root];
                    while let Some(block) = dfs.pop() {
                        dfs.extend(block.children.clone());
                        if block.tag.eq_ignore_ascii_case("ThingDef")
                            || block.tag.eq_ignore_ascii_case("Thing")
                            || block.tag.eq_ignore_ascii_case("li")
                        {
                            let name = block.get_val("Name").unwrap_or_default();
                            if name.is_empty() {
                                continue;
                            }
                            if let Some(disp) = block
                                .get_val("ThingName")
                                .or_else(|| block.get_val("DisplayName"))
                            {
                                if !disp.trim().is_empty() && disp.trim() != name {
                                    self.translations
                                        .insert(name.clone(), disp.trim().to_string());
                                }
                            }
                            if !block
                                .get_val("Type")
                                .or_else(|| block.get_val("ThingType"))
                                .unwrap_or_else(|| "Item".to_string())
                                .eq_ignore_ascii_case("Item")
                            {
                                continue;
                            }
                            let (rate, parent, other_world) = (
                                block
                                    .get_val("Rate")
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or(0),
                                block.get_val("Parent").unwrap_or_default(),
                                block
                                    .get_val("OtherWorld")
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or(0),
                            );
                            let (
                                mut label,
                                mut not_random,
                                mut item_kind,
                                mut has_be_made,
                                mut is_stuff,
                                mut stuff_categories,
                                mut element_kind,
                                mut fabao_suffix_len,
                                mut temp_max_add,
                                mut temp_min_add,
                            ) = (
                                String::new(),
                                0,
                                String::new(),
                                false,
                                false,
                                Vec::new(),
                                match block.get_val("ElementKind").unwrap_or_default().as_str() {
                                    "Jin" => 1,
                                    "Mu" => 2,
                                    "Shui" => 3,
                                    "Huo" => 4,
                                    "Tu" => 5,
                                    _ => 0,
                                },
                                1,
                                0.0,
                                0.0,
                            );
                            if let Some(item_node) = block
                                .children
                                .iter()
                                .find(|c| c.tag.eq_ignore_ascii_case("Item"))
                            {
                                label = item_node.get_val("Lable").unwrap_or_default();
                                not_random = item_node
                                    .get_val("NotRandom")
                                    .and_then(|c| c.parse().ok())
                                    .unwrap_or(0);
                                item_kind = item_node.get_val("Kind").unwrap_or_default();
                                if let Some(equip_node) = item_node
                                    .children
                                    .iter()
                                    .find(|c| c.tag.eq_ignore_ascii_case("Equip"))
                                {
                                    temp_max_add = equip_node
                                        .get_val("TemperatureMaxAdd")
                                        .and_then(|s| s.parse().ok())
                                        .unwrap_or(0.0);
                                    temp_min_add = equip_node
                                        .get_val("TemperatureMinAdd")
                                        .and_then(|s| s.parse().ok())
                                        .unwrap_or(0.0);
                                }
                                if let Some(be_mat_node) = item_node
                                    .children
                                    .iter()
                                    .find(|c| c.tag.eq_ignore_ascii_case("BeMaterial"))
                                {
                                    is_stuff = true;
                                    match be_mat_node
                                        .get_val("ElementKind")
                                        .unwrap_or_default()
                                        .as_str()
                                    {
                                        "Jin" => element_kind = 1,
                                        "Mu" => element_kind = 2,
                                        "Shui" => element_kind = 3,
                                        "Huo" => element_kind = 4,
                                        "Tu" => element_kind = 5,
                                        _ => {}
                                    }
                                }
                                if let Some(bemade_node) = item_node
                                    .children
                                    .iter()
                                    .find(|c| c.tag.eq_ignore_ascii_case("BeMade"))
                                {
                                    has_be_made = true;
                                    for c3 in bemade_node.get_list("StuffCategories") {
                                        let name_attr = c3
                                            .get_val("name")
                                            .or_else(|| c3.get_val("Name"))
                                            .unwrap_or_else(|| c3.text.clone());
                                        if !name_attr.is_empty() {
                                            stuff_categories.push(name_attr);
                                        }
                                    }
                                }
                                let suffix_raw =
                                    item_node.get_val("FabaoSuffix").unwrap_or_default();
                                if !suffix_raw.is_empty() {
                                    fabao_suffix_len = suffix_raw.split(',').count().max(1);
                                }
                            }
                            temp_thing_defs.insert(
                                name.clone(),
                                (
                                    parent,
                                    ThingDefData {
                                        name: name.clone(),
                                        rate,
                                        not_random,
                                        item_kind,
                                        label: label.clone(),
                                        has_be_made,
                                        is_stuff,
                                        stuff_categories,
                                        element_kind,
                                        fabao_suffix_len,
                                        temp_max_add,
                                        temp_min_add,
                                        other_world,
                                    },
                                ),
                            );
                            if !parsed_things.contains(&name) {
                                parsed_things.push(name.clone());
                            }
                        }
                    }
                }
            }
        }
        for name in parsed_things {
            if let Some((parent, def)) = temp_thing_defs.get(&name) {
                let (mut resolved, mut curr_parent, mut depth) = (def.clone(), parent.clone(), 0);
                while !curr_parent.is_empty() && depth < 10 {
                    if let Some((_, p_def)) = temp_thing_defs.get(&curr_parent) {
                        if resolved.label.is_empty() {
                            resolved.label = p_def.label.clone();
                        }
                        if resolved.item_kind.is_empty() {
                            resolved.item_kind = p_def.item_kind.clone();
                        }
                        if resolved.rate == 0 {
                            resolved.rate = p_def.rate;
                        }
                        if resolved.element_kind == 0 {
                            resolved.element_kind = p_def.element_kind;
                        }
                        if !resolved.has_be_made && p_def.has_be_made {
                            resolved.has_be_made = true;
                            resolved.stuff_categories = p_def.stuff_categories.clone();
                        }
                        if !resolved.is_stuff && p_def.is_stuff {
                            resolved.is_stuff = true;
                        }
                        if resolved.temp_max_add == 0.0 && resolved.temp_min_add == 0.0 {
                            resolved.temp_max_add = p_def.temp_max_add;
                            resolved.temp_min_add = p_def.temp_min_add;
                        }
                        if resolved.other_world == 0 {
                            resolved.other_world = p_def.other_world;
                        }
                        curr_parent = String::new();
                    } else {
                        break;
                    }
                    depth += 1;
                }
                if resolved.label.is_empty() {
                    resolved.label = "None".to_string();
                }
                self.thing_defs.insert(name.clone(), resolved.clone());
                if resolved.label != "None"
                    && resolved.not_random != 1
                    && resolved.not_random != 2
                    && !name.ends_with("Base")
                {
                    if !resolved.item_kind.eq_ignore_ascii_case("Equipment") || resolved.has_be_made
                    {
                        self.map_lable2item
                            .entry(resolved.label.clone())
                            .or_insert_with(Vec::new)
                            .push(name.clone());
                    }
                    if resolved.is_stuff {
                        self.map_lable2stuff
                            .entry(resolved.label.clone())
                            .or_insert_with(Vec::new)
                            .push(name.clone());
                    }
                }
            }
        }
        for list in self.map_lable2item.values_mut() {
            list.sort();
        }
        for list in self.map_lable2stuff.values_mut() {
            list.sort();
        }
    }
    fn load_npcs(&mut self, settings_dir: &Path) {
        let mut f_npc = vec![];
        walk_dir(&settings_dir.join("Jianghu/Npc"), &mut f_npc, "xml");
        f_npc.sort_by(|a, b| {
            a.to_string_lossy()
                .to_lowercase()
                .cmp(&b.to_string_lossy().to_lowercase())
        });
        let (mut parsed_names, mut temp_defs) = (Vec::new(), HashMap::new());
        for p in f_npc {
            if let Ok(content) = read_and_decrypt(&p) {
                for root in parse_xml(&content) {
                    let mut dfs = vec![root];
                    while let Some(block) = dfs.pop() {
                        dfs.extend(block.children.clone());
                        if block.tag.eq_ignore_ascii_case("JianghuNpcDef")
                            || block.tag.eq_ignore_ascii_case("JianghuNpc")
                            || block.tag.eq_ignore_ascii_case("li")
                            || block.tag.eq_ignore_ascii_case("Def")
                        {
                            let name = block.get_val("Name").unwrap_or_default();
                            if name.is_empty() {
                                continue;
                            }
                            let is_abstract = block
                                .get_val("Abstract")
                                .map(|s| s.eq_ignore_ascii_case("true") || s == "1")
                                .unwrap_or(false);
                            let parent = block
                                .get_val("ParentName")
                                .or_else(|| block.get_val("Parent"))
                                .unwrap_or_default();
                            let (last_name, first_name, title, school) = (
                                block.get_val("LastName").unwrap_or_default(),
                                block
                                    .get_val("FristName")
                                    .or_else(|| block.get_val("FirstName"))
                                    .unwrap_or_default(),
                                block
                                    .get_val("Title")
                                    .or_else(|| block.get_val("Titles"))
                                    .unwrap_or_default(),
                                block
                                    .get_val("School")
                                    .and_then(|c| c.parse().ok())
                                    .unwrap_or(-1),
                            );
                            let sex = match block.get_val("Sex").unwrap_or_default().as_str() {
                                "Male" | "1" => 1,
                                "Female" | "2" => 2,
                                "" => -1,
                                _ => 0,
                            };
                            let gong_level =
                                match block.get_val("GongLevel").unwrap_or_default().as_str() {
                                    "None" => 0,
                                    "Qi" => 1,
                                    "Dan1" => 2,
                                    "Dan2" => 3,
                                    "God" => 4,
                                    "God2" => 5,
                                    s => s.parse().unwrap_or(0),
                                };
                            let mut carry = None;
                            if let Some(carry_node) = block
                                .children
                                .iter()
                                .find(|c| c.tag.eq_ignore_ascii_case("Carry"))
                            {
                                let mut items = Vec::new();
                                for li in carry_node.get_list("li") {
                                    let thing_type = match li
                                        .get_val("ThingType")
                                        .unwrap_or_default()
                                        .as_str()
                                    {
                                        "Item" | "0" | "" => 0,
                                        "Esoterica" | "1" => 1,
                                        s => s.parse().unwrap_or(0),
                                    };
                                    let thing_def = li.get_val("ThingDef").unwrap_or_default();
                                    let stuff = li.get_val("Stuff").unwrap_or_default();
                                    if !thing_def.is_empty() || thing_type == 1 {
                                        items.push(DropItemData {
                                            thing_type,
                                            thing_def,
                                            stuff,
                                        });
                                    }
                                }
                                carry = Some(items);
                            } else if block.children.iter().any(|c| {
                                c.tag.eq_ignore_ascii_case("Carry") && c.children.is_empty()
                            }) {
                                carry = Some(Vec::new());
                            }
                            if !temp_defs.contains_key(&name) {
                                parsed_names.push(name.clone());
                            }
                            temp_defs.insert(
                                name.clone(),
                                TempNpcDef {
                                    name: name.clone(),
                                    parent,
                                    last_name,
                                    first_name,
                                    title,
                                    school,
                                    sex,
                                    gong_level,
                                    carry,
                                    is_abstract,
                                },
                            );
                        }
                    }
                }
            }
        }
        for name in parsed_names {
            if let Some(def) = temp_defs.get(&name) {
                let (mut resolved, mut curr_parent, mut depth) =
                    (def.clone(), def.parent.clone(), 0);
                while !curr_parent.is_empty() && depth < 10 {
                    if let Some(p_def) = temp_defs.get(&curr_parent) {
                        if resolved.carry.is_none() {
                            resolved.carry = p_def.carry.clone();
                        }
                        if resolved.school == -1 {
                            resolved.school = p_def.school;
                        }
                        if resolved.sex == -1 {
                            resolved.sex = p_def.sex;
                        }
                        if resolved.gong_level == 0 {
                            resolved.gong_level = p_def.gong_level;
                        }
                        if resolved.title.is_empty() {
                            resolved.title = p_def.title.clone();
                        }
                        curr_parent = p_def.parent.clone();
                    } else {
                        break;
                    }
                    depth += 1;
                }
                let new_def = JianghuNpcDef {
                    name: def.name.clone(),
                    school: if resolved.school == -1 {
                        0
                    } else {
                        resolved.school
                    },
                    sex: if resolved.sex == -1 { 0 } else { resolved.sex },
                    last_name: resolved.last_name,
                    first_name: resolved.first_name,
                    title: resolved.title,
                    gong_level: resolved.gong_level,
                    carry: resolved.carry.unwrap_or_default(),
                };
                self.name_to_index
                    .insert(def.name.clone(), self.jh_npcs_global.len());
                self.jh_npcs_global.push(new_def.clone());
                if !def.is_abstract {
                    let list = self
                        .school_to_names
                        .entry(new_def.school)
                        .or_insert_with(Vec::new);
                    if !list.contains(&def.name) {
                        list.push(def.name.clone());
                    }
                }
            }
        }

        // 构建 locationSeed → 模板名 映射（复制C# MakeJianghuNpcs逻辑）
        let mut jianghu_npcs: HashMap<i32, String> = HashMap::new();
        let mut jianghu_npc_index: HashMap<i32, i32> = HashMap::new();
        for &school_id in (1..=13).collect::<Vec<_>>().iter() {
            if let Some(list) = self.school_to_names.get(&school_id) {
                for template_name in list {
                    if let Some(idx) = self.name_to_index.get(template_name) {
                        let def = &self.jh_npcs_global[*idx];
                        if def.school > 0 {
                            let key = school_id * 100 + def.gong_level;
                            let num2 = *jianghu_npc_index.get(&key).unwrap_or(&0) + 1;
                            let location_seed =
                                school_id * 100_000 + def.gong_level * 10_000 + num2;
                            jianghu_npc_index.insert(key, num2);
                            jianghu_npcs.insert(location_seed, template_name.clone());
                        }
                    }
                }
            }
        }
        self.location_to_template = jianghu_npcs;
    }
}
