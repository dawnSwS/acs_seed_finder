#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct DropItemData {
    pub thing_type: i32,
    pub thing_def: String,
    pub stuff: String,
}
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct JianghuNpcDef {
    pub race: String,
    pub titles: String,
    pub carry_is_defined: bool,
    pub name: String,
    pub last_name: String,
    pub first_name: String,
    pub title: String,
    pub school: i32,
    pub sex: i32,
    pub gong_level: i32,
    pub carry: Vec<DropItemData>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ItemData {
    pub name: String,
    pub quality: f32,
    /// Fresh-item eligibility used solely to replay `EquipItemMaybeJingguai` RNG.
    pub(crate) tongling_eligible: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct NpcData {
    pub school_id: i32,
    pub stage: i32,
    pub index: i32,
    pub local_seed: i32,
    pub prediction_status: PredictionStatus,
    pub notes: Vec<String>,
    pub name: String,
    pub talismans: Vec<ItemData>,
    pub inventory: Vec<ItemData>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct SectData {
    pub sect_name: String,
    pub npcs: Vec<NpcData>,
}
#[derive(Debug, Clone, Default, serde::Deserialize)]
// These independent flags retain the game's Item/BeMade/BeMaterial semantics.
#[allow(clippy::struct_excessive_bools)]
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
    pub fabao_suffix: String,
    pub material_name_prefix: String,
    pub made_quality: f32,
    pub no_material_prefix: bool,
    pub temp_max_add: f32,
    pub temp_min_add: f32,
    pub other_world: i32,
    pub tongling_eligible: bool,
}
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ModifierPropertyData {
    pub name: String,
    pub add_v: f32,
    pub add_p: f32,
}
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ModifierDefData {
    #[serde(rename = "name")]
    pub _name: String,
    pub properties: Vec<ModifierPropertyData>,
}
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct SpellDefData {
    pub name: String,
    pub has_template: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct NamePoolData {
    pub prefix: Vec<String>,
    pub male_suffix: Vec<String>,
    pub female_suffix: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct SchoolData {
    pub rank: i32,
    pub id: i32,
    pub name: String,
    pub race: String,
    pub body_practice: bool,
}

/// Exact replay requires the world's mutable practice/cache state for cultivators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredictionStatus {
    Complete,
    RequiresWorldState,
}
