// Definition merges use indices collected from the same unchanged vector.
#![allow(clippy::indexing_slicing)]

use super::{
    crypto::read_and_decrypt,
    types::{
        DropItemData, JianghuNpcDef, ModifierDefData, ModifierPropertyData, NamePoolData,
        SchoolData, SpellDefData, ThingDefData,
    },
    xml::{parse_xml_checked, XmlElement},
};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

const EMBEDDED_DATA: &str = include_str!("../../assets/game_data.json");

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameData {
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
    pub name_pools: HashMap<String, NamePoolData>,
    pub schools: Vec<SchoolData>,
    pub generated_races: Vec<String>,
    pub special_names: HashMap<String, HashMap<String, Vec<String>>>,
    pub art_names: HashMap<String, Vec<String>>,
    pub auxiliary_tables: HashMap<String, Vec<Vec<String>>>,
    #[serde(skip)]
    pub effect_pool_counts: HashMap<(i32, i32, i32), i32>,
}

fn files_in(dir: &Path, extension: &str) -> Result<Vec<PathBuf>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    visit_files(dir, extension, &mut files)?;
    files.sort_by_key(|path| path.to_string_lossy().to_lowercase());
    Ok(files)
}

fn visit_files(dir: &Path, extension: &str, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))? {
        let path = entry.map_err(|error| error.to_string())?.path();
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        if metadata.is_dir() {
            visit_files(&path, extension, files)?;
        } else if metadata.is_file()
            && path
                .extension()
                .is_some_and(|value| value.eq_ignore_ascii_case(extension))
        {
            files.push(path);
        }
    }
    Ok(())
}

fn read_xml(path: &Path) -> Result<Vec<XmlElement>, String> {
    let content = read_and_decrypt(path).map_err(|error| format!("{}: {error}", path.display()))?;
    parse_xml_checked(&content).map_err(|error| format!("{}: {error}", path.display()))
}

/// Select definitions, never nested effect/property <li Name=...> records.
fn definitions(dir: &Path, tags: &[&str]) -> Result<Vec<XmlElement>, String> {
    let mut order = Vec::new();
    let mut nodes = HashMap::new();
    let item_namespace = tags.iter().any(|tag| tag.eq_ignore_ascii_case("ThingDef"));
    for path in files_in(dir, "xml")? {
        for root in read_xml(&path)? {
            collect_definitions(&root, tags, item_namespace, &mut order, &mut nodes);
        }
    }
    let mut resolved = HashMap::new();
    order
        .iter()
        .map(|name| resolve_definition(name, &nodes, &mut resolved, &mut HashSet::new()))
        .collect()
}

fn collect_definitions(
    root: &XmlElement,
    tags: &[&str],
    item_namespace: bool,
    order: &mut Vec<String>,
    nodes: &mut HashMap<String, XmlElement>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let is_definition = tags.iter().any(|tag| node.tag.eq_ignore_ascii_case(tag))
            && node.get_val("Name").is_some();
        if !is_definition {
            stack.extend(node.children.iter().rev());
            continue;
        }
        // ThingMgr keeps independent Type dictionaries: plants cannot replace items.
        if item_namespace && !is_item_definition(node) {
            continue;
        }
        insert_definition(node, order, nodes);
    }
}

fn is_item_definition(node: &XmlElement) -> bool {
    node.get_val("Type")
        .is_some_and(|kind| kind.eq_ignore_ascii_case("Item") || kind == "2")
}

fn insert_definition(
    node: &XmlElement,
    order: &mut Vec<String>,
    nodes: &mut HashMap<String, XmlElement>,
) {
    let name = node.get_val("Name").unwrap_or_default();
    if name.is_empty() {
        return;
    }
    if !nodes.contains_key(&name) {
        order.push(name.clone());
    }
    nodes.insert(name, node.clone());
}

fn resolve_definition(
    name: &str,
    nodes: &HashMap<String, XmlElement>,
    resolved: &mut HashMap<String, XmlElement>,
    visiting: &mut HashSet<String>,
) -> Result<XmlElement, String> {
    if let Some(node) = resolved.get(name) {
        return Ok(node.clone());
    }
    if !visiting.insert(name.to_owned()) {
        return Err(format!("definition inheritance cycle at {name}"));
    }
    let node = nodes
        .get(name)
        .ok_or_else(|| format!("missing definition parent {name}"))?;
    let parent = node
        .get_val("ParentName")
        .or_else(|| node.get_val("Parent"))
        .unwrap_or_default();
    let mut merged = if parent.is_empty() {
        node.clone()
    } else {
        merge_nodes(
            &resolve_definition(&parent, nodes, resolved, visiting)?,
            node,
        )
    };
    apply_definition_overrides(&mut merged, node);
    visiting.remove(name);
    resolved.insert(name.to_owned(), merged.clone());
    Ok(merged)
}

fn apply_definition_overrides(merged: &mut XmlElement, node: &XmlElement) {
    merged
        .attributes
        .retain(|key, _| !key.eq_ignore_ascii_case("Abstract"));
    if let Some(value) = node.get_val("Abstract") {
        merged.attributes.insert("Abstract".into(), value);
    }
    // GetDef initializes the parent before inheriting NoElementKind.
    if integer(merged, "NoElementKind") > 0 {
        merged
            .attributes
            .retain(|key, _| !key.eq_ignore_ascii_case("ElementKind"));
        merged
            .children
            .retain(|node| !node.tag.eq_ignore_ascii_case("ElementKind"));
        merged
            .attributes
            .insert("ElementKind".into(), "None".into());
    }
}

fn missing_enum_value(container: &str, field: &str, value: &str) -> bool {
    let enum_field = ["Lable", "ElementKind", "Sex", "GongLevel"]
        .iter()
        .any(|name| field.eq_ignore_ascii_case(name))
        || (field.eq_ignore_ascii_case("Kind") && container.eq_ignore_ascii_case("Item"))
        || (field.eq_ignore_ascii_case("Type")
            && ["ThingDef", "Thing"]
                .iter()
                .any(|name| container.eq_ignore_ascii_case(name)));
    enum_field && (value.eq_ignore_ascii_case("None") || value == "0")
}

fn collection_node(node: &XmlElement) -> bool {
    node.children.iter().any(|entry| {
        ["li", "Stuff", "Item"]
            .iter()
            .any(|tag| entry.tag.eq_ignore_ascii_case(tag))
    })
}

/// Matches GameUlt.InheritClass: recurse through classes, inherit empty Lists
/// and enum zero; explicit numeric zero and empty scalar strings remain values.
fn merge_nodes(parent: &XmlElement, child: &XmlElement) -> XmlElement {
    let mut merged = parent.clone();
    merged.tag.clone_from(&child.tag);
    if !child.text.is_empty() || child.children.is_empty() {
        merged.text.clone_from(&child.text);
    }
    merge_attributes(&mut merged, child);
    for node in &child.children {
        merge_child(&mut merged, child, node);
    }
    merged
}

fn merge_attributes(merged: &mut XmlElement, child: &XmlElement) {
    for (name, value) in &child.attributes {
        if missing_enum_value(&child.tag, name, value) {
            continue;
        }
        merged
            .attributes
            .retain(|key, _| !key.eq_ignore_ascii_case(name));
        merged.attributes.insert(name.clone(), value.clone());
    }
}

fn merge_child(merged: &mut XmlElement, child: &XmlElement, node: &XmlElement) {
    let matching: Vec<_> = merged
        .children
        .iter()
        .enumerate()
        .filter(|(_, other)| other.tag.eq_ignore_ascii_case(&node.tag))
        .map(|(index, _)| index)
        .collect();
    let Some(&index) = matching.first() else {
        merged.children.push(node.clone());
        return;
    };
    if missing_enum_value(&child.tag, &node.tag, &node.text) {
        return;
    }
    merged.children[index] = inherited_child(&merged.children[index], node, matching.len());
    for &extra in matching.iter().skip(1).rev() {
        merged.children.remove(extra);
    }
}

fn inherited_child(parent: &XmlElement, child: &XmlElement, matching_count: usize) -> XmlElement {
    let list_like = collection_node(child) || collection_node(parent) || matching_count > 1;
    let class_like = !parent.children.is_empty()
        || !child.children.is_empty()
        || ["Item", "BeMade", "BeMaterial", "Equip"]
            .iter()
            .any(|tag| child.tag.eq_ignore_ascii_case(tag));
    if list_like && child.children.is_empty() && child.text.is_empty() {
        parent.clone()
    } else if class_like && !list_like {
        merge_nodes(parent, child)
    } else {
        child.clone()
    }
}

fn integer(node: &XmlElement, name: &str) -> i32 {
    node.get_val(name)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}
fn number(node: &XmlElement, name: &str) -> f32 {
    node.get_val(name)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0.0)
}
fn abstract_def(node: &XmlElement) -> bool {
    node.get_val("Abstract")
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}
fn element(value: &str) -> i32 {
    match value {
        "Jin" => 1,
        "Mu" => 2,
        "Shui" => 3,
        "Huo" => 4,
        "Tu" => 5,
        "Chaos" => 6,
        _ => value.parse().unwrap_or(0),
    }
}
fn read_names(path: &Path) -> Result<Vec<String>, String> {
    let content = read_and_decrypt(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(parse_name_table(&content))
}

fn parse_name_table(content: &str) -> Vec<String> {
    content
        .trim_start_matches('\u{feff}')
        .split(['\r', '\n'])
        .filter_map(|line| {
            let word = line.split('\t').next().unwrap_or_default().trim();
            (!word.is_empty() && !word.starts_with("//")).then(|| word.to_owned())
        })
        .collect()
}

fn parse_thing(node: &XmlElement) -> Option<ThingDefData> {
    if abstract_def(node) || integer(node, "Disable") > 0 || !is_item_definition(node) {
        return None;
    }
    let item = node.child("Item")?;
    let made = item.child("BeMade");
    let material = item.child("BeMaterial");
    let equip = item.child("Equip");
    Some(ThingDefData {
        name: node.get_val("Name").unwrap_or_default(),
        rate: integer(node, "Rate"),
        not_random: integer(item, "NotRandom"),
        item_kind: item.get_val("Kind").unwrap_or_default(),
        label: item
            .get_val("Lable")
            .filter(|label| !label.is_empty() && label != "None" && label != "0")
            .unwrap_or_else(|| "Other".into()),
        has_be_made: made.is_some(),
        is_stuff: material.is_some(),
        stuff_categories: made.map_or_else(Default::default, |made| {
            made.get_list("StuffCategories")
                .into_iter()
                .filter_map(|stuff| {
                    let value = stuff.get_val("name").unwrap_or(stuff.text);
                    (!value.is_empty()).then_some(value)
                })
                .collect()
        }),
        element_kind: if integer(node, "NoElementKind") > 0 {
            0
        } else {
            node.get_val("ElementKind")
                .map_or(0, |value| element(&value))
        },
        fabao_suffix_len: item
            .get_val("FabaoSuffix")
            .filter(|value| !value.is_empty())
            .map_or(1, |value| value.split(',').count()),
        fabao_suffix: item.get_val("FabaoSuffix").unwrap_or_default(),
        material_name_prefix: material
            .and_then(|material| material.get_val("NamePrefix"))
            .unwrap_or_default(),
        made_quality: made.map_or(0.0, |made| number(made, "Quality")),
        no_material_prefix: made.is_some_and(|made| integer(made, "NoPrefix") > 0),
        temp_max_add: equip.map_or(0.0, |equip| number(equip, "TemperatureMaxAdd")),
        temp_min_add: equip.map_or(0.0, |equip| number(equip, "TemperatureMinAdd")),
        other_world: integer(node, "OtherWorld"),
        tongling_eligible: node
            .get_val("SelectAble")
            .is_none_or(|value| value.parse::<i32>().unwrap_or(0) > 0)
            && integer(node, "NotDoTongLing") != 1,
    })
}

fn parse_npc(node: &XmlElement) -> JianghuNpcDef {
    let carry_node = node.child("Carry");
    JianghuNpcDef {
        name: node.get_val("Name").unwrap_or_default(),
        race: node.get_val("Race").unwrap_or_default(),
        titles: node.get_val("Titles").unwrap_or_default(),
        title: node.get_val("Title").unwrap_or_default(),
        carry_is_defined: carry_node.is_some(),
        carry: carry_node.map_or_else(Default::default, |carry| {
            carry
                .children
                .iter()
                .filter(|node| node.tag.eq_ignore_ascii_case("li"))
                .map(parse_drop)
                .collect()
        }),
        last_name: node.get_val("LastName").unwrap_or_default(),
        first_name: node
            .get_val("FristName")
            .or_else(|| node.get_val("FirstName"))
            .unwrap_or_default(),
        school: integer(node, "School"),
        sex: sex_value(&node.get_val("Sex").unwrap_or_default()),
        gong_level: gong_level(&node.get_val("GongLevel").unwrap_or_default()),
    }
}

fn parse_drop(node: &XmlElement) -> DropItemData {
    let kind = node.get_val("ThingType").unwrap_or_default();
    DropItemData {
        thing_type: if kind == "Esoterica" || kind == "1" {
            1
        } else {
            kind.parse().unwrap_or(0)
        },
        thing_def: node.get_val("ThingDef").unwrap_or_default(),
        stuff: node.get_val("Stuff").unwrap_or_default(),
    }
}

fn sex_value(value: &str) -> i32 {
    match value {
        "Male" => 1,
        "Female" => 2,
        value => value.parse().unwrap_or(0),
    }
}

fn gong_level(value: &str) -> i32 {
    match value {
        "Qi" => 1,
        "Dan1" => 2,
        "Dan2" => 3,
        "God" => 4,
        "God2" => 5,
        value => value.parse().unwrap_or(0),
    }
}

impl GameData {
    pub fn try_load_from_dir(base_path: &Path) -> Result<Self, String> {
        let settings = if base_path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("Settings"))
        {
            base_path.to_path_buf()
        } else {
            base_path.join("Settings")
        };
        if !settings.is_dir() {
            return Err(format!("Settings 目录不存在: {}", settings.display()));
        }
        let mut data = Self::default();
        data.load_tables(&settings)?;
        // Base definitions already contain Chinese text. Other-language folders
        // must never overwrite those names implicitly.
        data.validate()?;
        Ok(data)
    }

    fn load_tables(&mut self, settings: &Path) -> Result<(), String> {
        self.load_names(settings)?;
        self.load_schools(settings)?;
        self.load_fabao_lengths(settings)?;
        self.load_modifiers(settings)?;
        self.load_spells(settings)?;
        self.load_things(settings)?;
        self.load_npcs(settings)?;
        self.load_generation_tables(settings)?;
        self.load_item_names(settings)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_required_definitions()?;
        self.validate_pool_sizes()?;
        self.validate_references()?;
        self.validate_generation_tables()
    }

    fn validate_required_definitions(&self) -> Result<(), String> {
        let human_names = [
            &self.names_prefix,
            &self.names_m_suffix,
            &self.names_f_suffix,
        ];
        if human_names.iter().any(|pool| pool.is_empty()) {
            return Err("NPC 姓名数据缺失，请使用完整 Settings 目录".into());
        }
        if self.thing_defs.is_empty() || self.map_lable2item.is_empty() {
            return Err("NPC 物品定义缺失，请使用完整游戏数据".into());
        }
        if !self.spells_list.iter().any(|spell| spell.has_template) {
            return Err("NPC 符箓定义缺失，请使用完整游戏数据".into());
        }
        if self.schools.is_empty() {
            return Err("NPC 门派配置缺失，请使用完整游戏数据".into());
        }
        Ok(())
    }

    fn validate_pool_sizes(&self) -> Result<(), String> {
        let max = usize::try_from(i32::MAX - 1).map_err(|error| error.to_string())?;
        let item_pools = self
            .map_lable2item
            .values()
            .chain(self.map_lable2stuff.values());
        if self.spells_list.len() > max || item_pools.into_iter().any(|pool| pool.len() > max) {
            return Err("NPC 数据池超过游戏的整数索引范围".into());
        }
        let oversized_def = self
            .thing_defs
            .values()
            .any(|def| def.stuff_categories.len() > max || def.fabao_suffix_len > max);
        let oversized_name = self.name_pools.values().any(|pool| {
            [&pool.prefix, &pool.male_suffix, &pool.female_suffix]
                .iter()
                .any(|names| names.len() > max)
        });
        if oversized_def || oversized_name || self.generated_races.len() > max {
            return Err("NPC 定义超过游戏的整数索引范围".into());
        }
        Ok(())
    }

    fn validate_references(&self) -> Result<(), String> {
        self.validate_npc_references()?;
        for name in self
            .map_lable2item
            .values()
            .chain(self.map_lable2stuff.values())
            .flatten()
        {
            if !self.thing_defs.contains_key(name) {
                return Err(format!("物品池引用未知物品: {name}"));
            }
        }
        let spell_names: HashSet<_> = self.spells_list.iter().map(|spell| &spell.name).collect();
        if spell_names.len() != self.spells_list.len() {
            return Err("NPC 法术列表重复".into());
        }
        Ok(())
    }

    fn validate_npc_references(&self) -> Result<(), String> {
        for (name, &index) in &self.name_to_index {
            if self
                .jh_npcs_global
                .get(index)
                .is_none_or(|npc| npc.name != *name)
            {
                return Err(format!("NPC 模板索引损坏: {name}"));
            }
        }
        for name in self.location_to_template.values() {
            if !self.name_to_index.contains_key(name) {
                return Err(format!("NPC 位置引用未知模板: {name}"));
            }
        }
        Ok(())
    }

    fn validate_generation_tables(&self) -> Result<(), String> {
        for (path, minimum_rows) in [
            ("Fight/SkillEffect.txt", 3),
            ("Display/FabaoName/Prefix.txt", 6),
            ("Display/FabaoName/Mid.txt", 3),
            ("Display/EsotericaName/ElementPrefix.txt", 5),
            ("Display/EsotericaName/GongPrefix.txt", 4),
            ("Display/EsotericaName/StoryPrefix.txt", 1),
            ("Display/EsotericaName/SkillMid.txt", 10),
            ("Display/EsotericaName/SkillSuffix.txt", 1),
        ] {
            self.validate_table(path, minimum_rows)?;
        }
        if self.effect_pool_counts.is_empty()
            || self.effect_pool_counts.values().any(|&count| count <= 0)
        {
            return Err("NPC 战斗特效随机池缺失或损坏".into());
        }
        self.validate_race_names()
    }

    fn validate_table(&self, path: &str, minimum_rows: usize) -> Result<(), String> {
        let rows = self
            .auxiliary_tables
            .get(path)
            .filter(|rows| rows.len() >= minimum_rows)
            .ok_or_else(|| format!("NPC 生成数据表缺失或不完整: {path}"))?;
        let invalid_row = rows
            .iter()
            .take(minimum_rows)
            .any(|row| row.first().is_none_or(|value| value.trim().is_empty()));
        if path != "Fight/SkillEffect.txt" && invalid_row {
            return Err(format!("NPC 生成数据表缺失或不完整: {path}"));
        }
        Ok(())
    }

    fn validate_race_names(&self) -> Result<(), String> {
        let school_races = self
            .schools
            .iter()
            .map(|school| school.race.as_str())
            .filter(|race| !race.is_empty() && !race.starts_with("RANDOM"));
        for race in self
            .generated_races
            .iter()
            .map(String::as_str)
            .chain(school_races)
        {
            let valid = self.name_pools.get(race).is_some_and(|pool| {
                [&pool.prefix, &pool.male_suffix, &pool.female_suffix]
                    .iter()
                    .all(|names| !names.is_empty())
            });
            if !valid {
                return Err(format!("NPC 种族姓名数据缺失: {race}"));
            }
        }
        Ok(())
    }

    pub fn load_embedded() -> Result<Self, String> {
        let mut data: Self = serde_json::from_str(EMBEDDED_DATA)
            .map_err(|error| format!("NPC 数据解析失败: {error}"))?;
        if let Some(rows) = data.auxiliary_tables.get("Fight/SkillEffect.txt") {
            data.effect_pool_counts = Self::effect_counts(rows);
        }
        data.validate()?;
        Ok(data)
    }

    pub fn translate(&self, key: &str) -> String {
        self.translations
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_owned())
    }

    fn load_names(&mut self, settings: &Path) -> Result<(), String> {
        for race in definitions(&settings.join("Npc/Race"), &["RaceDef", "Def"])? {
            let name = race.get_val("Name").unwrap_or_default();
            let table = |key: &str| -> Result<Vec<String>, String> {
                match race.get_val(key).filter(|value| !value.is_empty()) {
                    Some(path) => {
                        let path = settings.join("Display/NpcName").join(path);
                        if path.is_file() {
                            read_names(&path)
                        } else {
                            Ok(Vec::new())
                        }
                    }
                    None => Ok(Vec::new()),
                }
            };
            let pool = NamePoolData {
                prefix: table("NameGeneratorPrefix")?,
                male_suffix: table("NameGeneratorMaleSuffix")?,
                female_suffix: table("NameGeneratorFemaleSuffix")?,
            };
            if name == "Human" {
                self.names_prefix.clone_from(&pool.prefix);
                self.names_m_suffix.clone_from(&pool.male_suffix);
                self.names_f_suffix.clone_from(&pool.female_suffix);
            }
            if integer(&race, "GentrateNpc") > 0 {
                self.generated_races.push(name.clone());
            }
            self.name_pools.insert(name, pool);
        }
        Ok(())
    }

    fn load_schools(&mut self, settings: &Path) -> Result<(), String> {
        for path in files_in(&settings.join("World/Schools"), "txt")? {
            let content =
                read_and_decrypt(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            let mut lines = content.trim_start_matches('\u{feff}').lines();
            let headers: Vec<_> = lines.next().unwrap_or_default().split('\t').collect();
            for line in lines.skip(1) {
                let fields: Vec<_> = line.split('\t').collect();
                let field = |key: &str| {
                    headers
                        .iter()
                        .position(|header| *header == key)
                        .and_then(|index| fields.get(index))
                        .copied()
                        .unwrap_or_default()
                        .trim()
                };
                let id = field("ID").parse::<i32>().unwrap_or(0);
                if id <= 0 {
                    continue;
                }
                let school = SchoolData {
                    rank: field("Rank").parse::<i32>().unwrap_or(0),
                    id,
                    name: field("Name").to_owned(),
                    race: field("Race").to_owned(),
                    body_practice: field("BodyPractice").parse::<i32>().unwrap_or(0) > 0,
                };
                if let Some(existing) = self.schools.iter_mut().find(|school| school.id == id) {
                    *existing = school;
                } else {
                    self.schools.push(school);
                }
            }
        }
        Ok(())
    }

    fn load_fabao_lengths(&mut self, settings: &Path) -> Result<(), String> {
        for (path, lengths) in [
            (
                "Display/FabaoName/Prefix.txt",
                &mut self.fabao_prefix_lengths,
            ),
            ("Display/FabaoName/Mid.txt", &mut self.fabao_mid_lengths),
        ] {
            let content = read_and_decrypt(&settings.join(path))
                .map_err(|error| format!("{path}: {error}"))?;
            for (index, line) in content.lines().enumerate() {
                if let Some(words) = line.split('\t').next() {
                    lengths.insert(
                        i32::try_from(index + 1).map_err(|error| error.to_string())?,
                        i32::try_from(words.split(',').count())
                            .map_err(|error| error.to_string())?,
                    );
                }
            }
        }
        Ok(())
    }

    fn load_modifiers(&mut self, settings: &Path) -> Result<(), String> {
        for node in definitions(
            &settings.join("Modifiers"),
            &["Modifier", "ModifierDef", "Def"],
        )? {
            let name = node.get_val("Name").unwrap_or_default();
            let properties = node
                .child("Properties")
                .map_or_else(Default::default, |properties| {
                    properties
                        .children
                        .iter()
                        .filter_map(|property| {
                            property.get_val("Name").map(|name| ModifierPropertyData {
                                name,
                                add_v: number(property, "AddV"),
                                add_p: number(property, "AddP"),
                            })
                        })
                        .collect()
                });
            self.modifier_defs.insert(
                name.clone(),
                ModifierDefData {
                    _name: name,
                    properties,
                },
            );
        }
        Ok(())
    }

    fn load_spells(&mut self, settings: &Path) -> Result<(), String> {
        for node in definitions(
            &settings.join("Practice/Spell"),
            &["Spell", "SpellDef", "Def"],
        )? {
            if abstract_def(&node) {
                continue;
            }
            let name = node.get_val("Name").unwrap_or_default();
            if let Some(display) = node
                .get_val("DisplayName")
                .filter(|value| !value.is_empty())
            {
                self.translations.insert(name.clone(), display);
            }
            self.spells_list.push(SpellDefData {
                name,
                has_template: node
                    .get_val("Template")
                    .is_some_and(|value| !value.is_empty()),
            });
        }
        Ok(())
    }

    fn load_things(&mut self, settings: &Path) -> Result<(), String> {
        for node in definitions(&settings.join("ThingDef"), &["ThingDef", "Thing", "Def"])? {
            let Some(def) = parse_thing(&node) else {
                continue;
            };
            if let Some(display) = node
                .get_val("ThingName")
                .or_else(|| node.get_val("DisplayName"))
                .filter(|value| !value.is_empty())
            {
                self.translations.insert(def.name.clone(), display);
            }
            self.insert_thing(def);
        }
        Ok(())
    }

    fn insert_thing(&mut self, def: ThingDefData) {
        let random = def.not_random != 1 && def.not_random != 2 && !def.name.ends_with("Base");
        if random {
            if !def.item_kind.eq_ignore_ascii_case("Equipment") || def.has_be_made {
                self.map_lable2item
                    .entry(def.label.clone())
                    .or_default()
                    .push(def.name.clone());
            }
            if def.is_stuff {
                self.map_lable2stuff
                    .entry(def.label.clone())
                    .or_default()
                    .push(def.name.clone());
            }
        }
        self.thing_defs.insert(def.name.clone(), def);
    }

    fn load_npcs(&mut self, settings: &Path) -> Result<(), String> {
        for node in definitions(
            &settings.join("Jianghu/Npc"),
            &["JianghuNpcDef", "JianghuNpc", "Def"],
        )? {
            if !abstract_def(&node) {
                self.insert_npc(parse_npc(&node))?;
            }
        }
        Ok(())
    }

    fn insert_npc(&mut self, def: JianghuNpcDef) -> Result<(), String> {
        self.name_to_index
            .insert(def.name.clone(), self.jh_npcs_global.len());
        self.school_to_names
            .entry(def.school)
            .or_default()
            .push(def.name.clone());
        let index_count = self.school_to_names[&def.school]
            .iter()
            .filter(|other| {
                self.name_to_index
                    .get(*other)
                    .and_then(|index| self.jh_npcs_global.get(*index))
                    .is_some_and(|other| other.gong_level == def.gong_level)
            })
            .count();
        let index = i32::try_from(index_count + 1).map_err(|error| error.to_string())?;
        if def.school > 0 {
            self.location_to_template.insert(
                def.school * 100_000 + def.gong_level * 10_000 + index,
                def.name.clone(),
            );
        }
        self.jh_npcs_global.push(def);
        Ok(())
    }
}

impl GameData {
    fn load_item_names(&mut self, settings: &Path) -> Result<(), String> {
        for path in files_in(&settings.join("Display/SpecialName"), "xml")? {
            for root in read_xml(&path)? {
                for node in root
                    .descendants()
                    .filter(|node| node.tag.eq_ignore_ascii_case("SpecialItem"))
                {
                    self.insert_special_names(node);
                }
            }
        }
        self.load_art_names(settings)
    }

    fn insert_special_names(&mut self, node: &XmlElement) {
        let name = node.get_val("Name").unwrap_or_default();
        for stuff in &node.children {
            let list = self
                .special_names
                .entry(name.clone())
                .or_default()
                .entry(stuff.tag.clone())
                .or_default();
            for entry in &stuff.children {
                if let Some(name) = entry.get_val("Name") {
                    if !list.contains(&name) {
                        list.push(name);
                    }
                }
            }
        }
    }

    fn load_art_names(&mut self, settings: &Path) -> Result<(), String> {
        for node in definitions(&settings.join("Display/ArtItem"), &["ArtItem"])? {
            let name = node.get_val("Name").unwrap_or_default();
            let names = node.child("List").map_or_else(Default::default, |list| {
                list.children
                    .iter()
                    .filter_map(|entry| entry.get_val("Name"))
                    .collect()
            });
            self.art_names.insert(name, names);
        }
        Ok(())
    }

    fn load_generation_tables(&mut self, settings: &Path) -> Result<(), String> {
        const TABLES: [&str; 9] = [
            "Fight/SkillEffect.txt",
            "Display/FabaoName/Prefix.txt",
            "Display/FabaoName/Mid.txt",
            "Display/EsotericaName/ElementPrefix.txt",
            "Display/EsotericaName/GongPrefix.txt",
            "Display/EsotericaName/StoryPrefix.txt",
            "Display/EsotericaName/SkillMid.txt",
            "Display/EsotericaName/SkillSuffix.txt",
            "Display/EsotericaName/Suffix.txt",
        ];
        for name in TABLES {
            let path = settings.join(name);
            if !path.exists() {
                continue;
            }
            let text =
                read_and_decrypt(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            let normalized_text = text.replace("\r\n", "\n").replace('\r', "\n");
            let rows: Vec<Vec<String>> = normalized_text
                .trim_start_matches('\u{feff}')
                .lines()
                .map(|line| line.split('\t').map(str::to_owned).collect())
                .collect();
            if name == "Fight/SkillEffect.txt" {
                self.effect_pool_counts = Self::effect_counts(&rows);
            }
            self.auxiliary_tables.insert(name.to_owned(), rows);
        }
        Ok(())
    }
    fn effect_counts(rows: &[Vec<String>]) -> HashMap<(i32, i32, i32), i32> {
        let mut counts = HashMap::new();
        let header = rows.first().map_or_else(Default::default, Vec::as_slice);
        let columns: Vec<_> = ["ID", "Feel", "Shape", "Element"]
            .iter()
            .map(|name| header.iter().position(|column| column == name))
            .collect();
        if !columns.iter().all(Option::is_some) {
            return counts;
        }
        let mut ids = HashSet::new();
        for row in rows.iter().skip(2) {
            let values: Vec<i32> = columns
                .iter()
                .map(|column| {
                    column
                        .and_then(|index| row.get(index))
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0)
                })
                .collect();
            if values[0] != 0 && ids.insert(values[0]) {
                *counts.entry((values[1], values[2], values[3])).or_default() += 1;
            }
        }
        counts
    }
}
