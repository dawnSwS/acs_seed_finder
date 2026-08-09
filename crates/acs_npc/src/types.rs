#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DropItemData {
    pub thing_type: i32,
    pub thing_def: String,
    pub stuff: String,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct JianghuNpcDef {
    pub name: String,
    pub last_name: String,
    pub first_name: String,
    pub title: String,
    pub school: i32,
    pub sex: i32,
    pub gong_level: i32,
    pub carry: Vec<DropItemData>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ItemData {
    pub name: String,
    pub quality: f32,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NpcData {
    pub name: String,
    pub talismans: Vec<ItemData>,
    pub inventory: Vec<ItemData>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SectData {
    pub sect_name: String,
    pub npcs: Vec<NpcData>,
}
#[derive(Clone)]
pub(crate) struct TempNpcDef {
    pub name: String,
    pub parent: String,
    pub last_name: String,
    pub first_name: String,
    pub title: String,
    pub school: i32,
    pub sex: i32,
    pub gong_level: i32,
    pub carry: Option<Vec<DropItemData>>,
    pub is_abstract: bool,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ThingDefData {
    pub name: String,
    pub rate: i32,
    pub not_random: i32,
    pub item_kind: String,
    pub label: String,
    pub has_be_made: bool,
    pub is_stuff: bool,
    pub stuff_categories: Vec<String>,
    pub element_kind: i32,
    pub fabao_suffix_len: usize,
    pub temp_max_add: f32,
    pub temp_min_add: f32,
    pub other_world: i32,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ModifierPropertyData {
    pub name: String,
    pub add_v: f32,
    pub add_p: f32,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ModifierDefData {
    pub name: String,
    pub properties: Vec<ModifierPropertyData>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SpellDefData {
    pub name: String,
    pub has_template: bool,
}
