// C# reference parity requires signed/unsigned reinterpretation, truncating
// float casts and indexes selected by bounded game RNG ranges. Empty pools and
// external template indexes are checked; independent bit-level oracle fixtures
// verify these arithmetic boundaries instead of changing the game's semantics.
#![allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::indexing_slicing
)]

use crate::core::rng::{DotNetRandom, GRandom};
use crate::npc::{
    constants::{get_rich_data, CLOTHES_STUFF, S_ITEM_LABELS},
    data::GameData,
    items::{
        random_item_by_defname, random_item_by_defname_with_naming_rng,
        random_item_by_lable_with_season, random_item_defname,
    },
    rng_state::RngState,
    types::{DropItemData, ItemData, JianghuNpcDef, NpcData, PredictionStatus, SectData},
};

const DAO_MODIFIERS: [&str; 9] = [
    "Modifier_SpNpc_Ling",
    "Modifier_SpNpc_Shield",
    "Modifier_SpNpc_FabaoAtk",
    "Modifier_SpNpc_FabaoLing",
    "Modifier_SpNpc_FabaoSpeed",
    "Modifier_SpNpc_FabaoDisp",
    "Modifier_SpNpc_FabaoNum",
    "Modifier_SpNpc_FightSkillAtk",
    "Modifier_SpNpc_FightSkillCD",
];

#[inline]
fn pull_random_spell(d: &GameData, rng_sys: &mut dyn RngState) -> String {
    if d.spells_list.is_empty() {
        return String::new();
    }
    let mut idx = rng_sys.next_int(1, d.spells_list.len() as i32 + 1) as usize - 1;
    for _ in 0..d.spells_list.len() {
        if d.spells_list[idx].has_template {
            return d.spells_list[idx].name.clone();
        }
        idx = (idx + 1) % d.spells_list.len();
    }
    String::new()
}

fn get_jh_npc_local_seed(mut school: i32, rlevel: i32, index: i32) -> i32 {
    if school < 0 {
        school = 999;
    } else if school > 998 {
        school = 998;
    }
    school
        .wrapping_mul(100_000)
        .wrapping_add(rlevel.wrapping_mul(10_000))
        .wrapping_add(index)
}

/// Parameters read from the game's world rather than encoded in its seed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NpcGenerationSettings {
    /// `g_emSeason`: None=0, Spring=1, Summer=2, Autumn=3, Winter=4.
    pub season: i32,
    pub fight_difficult_coefficient: f32,
    pub fight_mode: bool,
}

impl Default for NpcGenerationSettings {
    fn default() -> Self {
        Self {
            season: 1,
            fight_difficult_coefficient: 1.0,
            fight_mode: false,
        }
    }
}

impl NpcGenerationSettings {
    pub fn validate(self) -> Result<Self, String> {
        if !(0..=4).contains(&self.season) {
            return Err("NPC 季节参数必须为 0..4".to_owned());
        }
        if !self.fight_difficult_coefficient.is_finite()
            || self.fight_difficult_coefficient <= 0.0
            || self.fight_difficult_coefficient > 100.0
        {
            return Err("NPC 战斗难度系数必须为 0..100 之间的有限正数".to_owned());
        }
        Ok(self)
    }
}

// Each stage retains the reference game's RNG channels and call order.
pub fn extract_npc_with_settings(
    school: i32,
    rlevel: i32,
    index: i32,
    world_seed: i32,
    d: &GameData,
    settings: NpcGenerationSettings,
) -> NpcData {
    let mut generator = NpcGenerator::new(school, rlevel, index, world_seed, d, settings);
    generator.generate_equipment();
    generator.discard_runtime_seed_items();
    generator.generate_school_bonus();
    generator.generate_carry();
    generator.finish()
}

struct NpcGenerator<'a> {
    data: &'a GameData,
    settings: NpcGenerationSettings,
    school: i32,
    stage: i32,
    index: i32,
    local_seed: i32,
    seed: i32,
    sex: i32,
    equipment_level: i32,
    richness: i32,
    dao: bool,
    name: String,
    carry: Vec<DropItemData>,
    jianghu: GRandom,
    system: DotNetRandom,
    inventory: Vec<ItemData>,
    talismans: Vec<ItemData>,
    notes: Vec<String>,
}

impl<'a> NpcGenerator<'a> {
    fn new(
        school: i32,
        stage: i32,
        index: i32,
        world_seed: i32,
        data: &'a GameData,
        settings: NpcGenerationSettings,
    ) -> Self {
        let local_seed = get_jh_npc_local_seed(school, stage, index);
        let seed = world_seed.wrapping_add(local_seed);
        let template = npc_template(data, local_seed);
        let mut jianghu = GRandom::new(seed as u32);
        let sex = npc_sex(template, &mut jianghu);
        let name = npc_name(data, template, school, sex, seed, &mut jianghu);
        let gong_level = npc_gong_level(template, stage);
        let carry = npc_carry(data, template, gong_level, seed);
        // Equipment always starts a fresh Jianghu channel after identity generation.
        jianghu = GRandom::new(seed as u32);
        let equipment_level = equipment_level(stage, &mut jianghu);
        let richness = jianghu.rand_range(0, 5);
        let dao = !data
            .schools
            .iter()
            .find(|s| s.id == school)
            .map_or(school == 12, |s| s.body_practice);
        Self {
            data,
            settings,
            school,
            stage,
            index,
            local_seed,
            seed,
            sex,
            equipment_level,
            richness,
            dao,
            name,
            carry,
            jianghu,
            system: DotNetRandom::new(seed),
            inventory: Vec::new(),
            talismans: Vec::new(),
            notes: prediction_notes(seed, stage, dao),
        }
    }

    fn generate_equipment(&mut self) {
        if self.stage == 5 {
            self.generate_elder_equipment();
        } else {
            self.generate_ordinary_equipment();
        }
    }

    fn generate_elder_equipment(&mut self) {
        let fabao_count = elder_fabao_count(
            self.data,
            self.seed,
            self.dao,
            self.settings.fight_difficult_coefficient,
        );
        self.jianghu = GRandom::new(self.seed as u32);
        self.system = DotNetRandom::new(self.seed);
        if self.dao {
            self.advance_elder_skills();
        }
        self.generate_elder_clothes();
        if self.dao {
            self.generate_elder_fabao(fabao_count);
            for _ in 0..3 {
                if let Some(item) =
                    random_spell_item(self.data, None, Some(2), Some(1.0), &mut self.system)
                {
                    self.talismans.push(item);
                }
            }
        }
    }

    fn advance_elder_skills(&mut self) {
        let _ = self.jianghu.rand_range(250_000, 750_000);
        let count = self.jianghu.rand_range(2, 6);
        for _ in 0..count {
            let skill =
                advance_fight_skill_rng(self.data, &mut self.jianghu, self.settings.fight_mode);
            let _ = self.system.next_int(0, 8);
            advance_fight_skill_name_rng(self.data, skill, &mut self.jianghu);
        }
    }

    fn generate_elder_clothes(&mut self) {
        let richness = get_rich_data(4);
        let stuff = CLOTHES_STUFF[self.system.next_int(0, CLOTHES_STUFF.len() as i32) as usize];
        for (label, material, season) in [
            ("Clothes", Some(stuff), self.settings.season),
            ("Trousers", Some(stuff), self.settings.season),
            ("Weapon", None, 0),
        ] {
            let quality = self.system.next_flt(richness.0, richness.1);
            self.equip_item(label, 11, 11, quality, material, season);
        }
    }

    fn generate_elder_fabao(&mut self, count: i32) {
        for _ in 0..count {
            self.equip_item("FightFabao", 12, 12, 0.0, None, 0);
        }
    }

    fn generate_ordinary_equipment(&mut self) {
        let richness = get_rich_data(self.richness);
        if self.wears_clothes() {
            self.equip_ordinary_item("Clothes", richness, self.settings.season);
        }
        self.equip_ordinary_item("Trousers", richness, self.settings.season);
        let weapon_rate = if self.richness <= 2 { 0.5 } else { 1.0 };
        if self.system.random_rate(weapon_rate) {
            self.equip_ordinary_item("Weapon", richness, 0);
        }
        self.generate_ordinary_bonus(richness);
    }

    fn wears_clothes(&mut self) -> bool {
        let no_clothes = self.equipment_level == 0 && self.school == 11;
        !no_clothes && (self.sex == 2 || self.system.random_rate(0.8))
    }

    fn generate_ordinary_bonus(&mut self, richness: (f32, f32, i32, i32)) {
        if self.dao && self.system.random_rate(0.001) {
            self.equip_ordinary_item("TreasureFabao", richness, 0);
        }
        if self.richness >= 3 && self.system.random_rate(0.1) {
            self.equip_item("Tool", 0, 5, 0.0, None, 0);
        }
        self.generate_ordinary_talisman();
    }

    fn generate_ordinary_talisman(&mut self) {
        if self.dao && self.richness >= 3 && self.system.random_rate(0.01) {
            if let Some(item) = random_spell_item(self.data, None, None, None, &mut self.system) {
                advance_awaken_rng(&item, &mut self.system);
                self.talismans.push(item);
            }
        }
    }

    fn equip_ordinary_item(&mut self, label: &str, richness: (f32, f32, i32, i32), season: i32) {
        let quality = self.system.next_flt(richness.0, richness.1);
        self.equip_item(label, richness.2, richness.3, quality, None, season);
    }

    fn equip_item(
        &mut self,
        label: &str,
        min: i32,
        max: i32,
        quality: f32,
        stuff: Option<&str>,
        season: i32,
    ) {
        if let Some(item) = random_item_by_lable_with_season(
            self.data,
            label,
            min,
            max,
            quality,
            stuff,
            season,
            &mut self.system,
        ) {
            advance_awaken_rng(&item, &mut self.system);
            self.inventory.push(item);
        }
    }

    fn discard_runtime_seed_items(&mut self) {
        if runtime_seed(self.seed) {
            self.inventory.clear();
            self.talismans.clear();
        }
    }

    fn generate_school_bonus(&mut self) {
        if self.school == 11 {
            for spell in ["Spell_TempLow1", "Spell_TempHigh1"] {
                let quality = self.jianghu.rand_float(0.6, 0.8);
                if let Some(item) = random_spell_item(
                    self.data,
                    Some(spell),
                    None,
                    Some(quality),
                    &mut self.system,
                ) {
                    self.talismans.push(item);
                }
            }
        }
    }

    fn naming_state_known(&self) -> bool {
        (self.stage == 0 || (self.stage == 5 && self.dao)) && !runtime_seed(self.seed)
    }

    fn advance_elder_penalty(&mut self, naming_known: bool) {
        if self.stage != 5 || !self.dao || self.school < 0 || !naming_known {
            return;
        }
        let rank = self
            .data
            .schools
            .iter()
            .find(|s| s.id == self.school)
            .map_or(0, |s| s.rank);
        let (chance, minimum, maximum) = elder_penalty_range(rank);
        let _ = self.system.next_flt(minimum, maximum);
        let chance_seed = if self.local_seed == 0 {
            self.seed
        } else {
            self.local_seed
        };
        let mut chance_rng = GRandom::new(chance_seed as u32);
        if chance_rng.random_rate(chance) {
            let _ = self.system.next_int(144, 432);
        }
    }

    fn generate_carry(&mut self) {
        let mut naming_known = self.naming_state_known();
        self.advance_elder_penalty(naming_known);
        // Take the owned list to avoid cloning it or borrowing self across generation.
        for (i, carry) in std::mem::take(&mut self.carry).iter().enumerate() {
            match carry.thing_type {
                0 => self.generate_carry_item(carry, i, naming_known),
                1 => {
                    naming_known = false;
                    self.note_once(
                        "随机秘录",
                        "随机秘录的名称与属性需要存档中的秘录生成器状态；未知秘录未列入结果。",
                    );
                }
                _ => {}
            }
        }
    }

    fn generate_carry_item(&mut self, carry: &DropItemData, index: usize, naming_known: bool) {
        let mut item_rng = GRandom::new((self.seed as u32).wrapping_add(index as u32));
        if !naming_known && item_has_random_name(self.data, &carry.thing_def) {
            self.note_once(
                "携带物品",
                "携带物品的随机别名或艺术品名称依赖运行时随机状态；对应物品未列入结果。",
            );
            return;
        }
        if let Some(item) = random_item_by_defname_with_naming_rng(
            self.data,
            &carry.thing_def,
            Some(&carry.stuff),
            0,
            12,
            -1.0,
            &mut item_rng,
            &mut self.system,
        ) {
            // A blank spell paper remains inventory rather than an enchanted talisman.
            self.inventory.push(item);
        }
    }

    fn note_once(&mut self, prefix: &str, note: &str) {
        if !self.notes.iter().any(|note| note.starts_with(prefix)) {
            self.notes.push(note.to_owned());
        }
    }

    fn finish(self) -> NpcData {
        NpcData {
            school_id: self.school,
            stage: self.stage,
            index: self.index,
            local_seed: self.local_seed,
            name: self.name,
            talismans: self.talismans,
            inventory: self.inventory,
            prediction_status: if self.notes.is_empty() {
                PredictionStatus::Complete
            } else {
                PredictionStatus::RequiresWorldState
            },
            notes: self.notes,
        }
    }
}

fn npc_template(data: &GameData, local_seed: i32) -> Option<&JianghuNpcDef> {
    data.location_to_template
        .get(&local_seed)
        .and_then(|name| data.name_to_index.get(name.trim_start_matches("_DIE_")))
        .and_then(|&index| data.jh_npcs_global.get(index))
}

fn npc_sex(template: Option<&JianghuNpcDef>, rng: &mut GRandom) -> i32 {
    let sex = template.map_or(0, |def| def.sex);
    if sex <= 0 {
        rng.rand_range(1, 3)
    } else {
        sex
    }
}

fn npc_name(
    data: &GameData,
    template: Option<&JianghuNpcDef>,
    school: i32,
    sex: i32,
    seed: i32,
    rng: &mut GRandom,
) -> String {
    let mut last = template.map_or_else(String::new, |def| def.last_name.clone());
    let mut first = template.map_or_else(String::new, |def| def.first_name.clone());
    if last.is_empty() || first.is_empty() {
        *rng = GRandom::new(seed as u32);
        let race = npc_race(data, template, school, rng);
        let (prefixes, suffixes) = name_components(data, race, sex);
        // Both lookups run even when only one component is absent.
        let random_last = random_name_component(prefixes, rng);
        let random_first = random_name_component(suffixes, rng);
        if last.is_empty() {
            last = random_last;
        }
        if first.is_empty() {
            first = random_first;
        }
    }
    let mut name = format!("{last}{first}");
    if name.is_empty() {
        name.push_str("無名氏");
    }
    let title = npc_title(template, seed, rng);
    if !title.is_empty() {
        name = format!("【{title}】{name}");
    }
    name
}

fn npc_race<'a>(
    data: &'a GameData,
    template: Option<&'a JianghuNpcDef>,
    school: i32,
    rng: &mut GRandom,
) -> &'a str {
    let school_race = data
        .schools
        .iter()
        .find(|s| s.id == school)
        .map(|s| s.race.as_str())
        .filter(|race| !race.is_empty());
    let rule = school_race.unwrap_or_else(|| template.map_or("Human", |def| def.race.as_str()));
    random_race(data, rule, rng)
}

fn name_components<'a>(data: &'a GameData, race: &str, sex: i32) -> (&'a [String], &'a [String]) {
    if let Some(pool) = data.name_pools.get(race) {
        return (
            &pool.prefix,
            gender_suffix(&pool.male_suffix, &pool.female_suffix, sex),
        );
    }
    if race == "Human" {
        (
            &data.names_prefix,
            gender_suffix(&data.names_m_suffix, &data.names_f_suffix, sex),
        )
    } else {
        (&[], &[])
    }
}

fn gender_suffix<'a>(male: &'a [String], female: &'a [String], sex: i32) -> &'a [String] {
    if sex == 1 {
        male
    } else {
        female
    }
}

fn npc_title<'a>(template: Option<&'a JianghuNpcDef>, seed: i32, rng: &mut GRandom) -> &'a str {
    let titles = template.map_or("", |def| def.titles.as_str());
    if titles.is_empty() {
        return template.map_or("", |def| def.title.as_str());
    }
    let titles: Vec<&str> = titles.split(',').collect();
    *rng = GRandom::new(seed as u32);
    titles[rng.rand_range(0, titles.len() as i32) as usize]
}

fn npc_gong_level(template: Option<&JianghuNpcDef>, stage: i32) -> i32 {
    let level = template.map_or(0, |def| def.gong_level);
    if level != 0 {
        level
    } else if (1..=5).contains(&stage) {
        stage
    } else {
        0
    }
}

fn npc_carry(
    data: &GameData,
    template: Option<&JianghuNpcDef>,
    level: i32,
    seed: i32,
) -> Vec<DropItemData> {
    let carry = template.map_or_else(Vec::new, |def| def.carry.clone());
    if !carry.is_empty() {
        return carry;
    }
    let mut rng = GRandom::new((seed as u32).wrapping_add(3));
    let count = rng.rand_range(1, level + 1);
    (0..count)
        .filter_map(|_| random_carry(data, level, &mut rng))
        .collect()
}

fn random_carry(data: &GameData, level: i32, rng: &mut GRandom) -> Option<DropItemData> {
    if rng.random_rate(0.3) && level > 0 {
        return Some(DropItemData {
            thing_type: 1,
            thing_def: String::new(),
            stuff: String::new(),
        });
    }
    let label = S_ITEM_LABELS[rng.rand_range(0, 24) as usize];
    let (min, max) = if level <= 0 {
        (1, 5)
    } else {
        (4, 5.max(level * 3))
    };
    random_item_defname(data, label, min, max, rng).map(|thing_def| DropItemData {
        thing_type: 0,
        thing_def,
        stuff: String::new(),
    })
}

fn equipment_level(stage: i32, rng: &mut GRandom) -> i32 {
    match stage {
        1 => rng.rand_range(1, 4),
        2 => rng.rand_range(4, 7),
        3 => rng.rand_range(7, 10),
        4 => rng.rand_range(10, 12),
        5 => 12,
        _ => 0,
    }
}

fn runtime_seed(seed: i32) -> bool {
    seed == 0 || seed == i32::MIN
}

fn prediction_notes(seed: i32, stage: i32, dao: bool) -> Vec<String> {
    let mut notes = Vec::new();
    if seed == i32::MIN {
        notes.push(
            "原版 NPC 随机生成器不能处理合成种子 i32::MIN；此结果需要游戏状态验证。".to_owned(),
        );
    }
    if seed == 0 {
        notes.push(
            "游戏会将 NPC 合成种子 0 替换为运行时随机种子；对应随机装备和符箓需要存档状态。"
                .to_owned(),
        );
    }
    if stage > 0 && stage < 5 && dao {
        notes.push(
            "普通修士的功法、技能树和额外符箓取决于存档状态；这里只列出种子可确定的物品。"
                .to_owned(),
        );
    }
    if stage > 0 && !dao {
        notes.push(
            "体修的器官装备与真炁生成依赖运行时修炼状态；这里只列出种子可确定的物品。".to_owned(),
        );
    }
    notes
}

fn elder_fabao_count(data: &GameData, seed: i32, dao: bool, coefficient: f32) -> i32 {
    let mut rng = DotNetRandom::new(seed);
    let count = if dao { 9 } else { 6 };
    let mut available: Vec<usize> = (0..count).collect();
    let (mut add_v, mut add_p) = modifier_fabao_effect(data, "Modifier_SpNpc_BasePropertie", 1.0);
    for index in 0..(if dao { 4 } else { 3 }) {
        let selected = rng.next_int(0, (count - index) as i32) as usize;
        let scale = if index == 0 {
            rng.next_int(300, 501)
        } else {
            rng.next_int(100, 201)
        } as f32
            / 100.0;
        if dao {
            let effect = modifier_fabao_effect(
                data,
                DAO_MODIFIERS[available[selected]],
                scale * coefficient,
            );
            add_v += effect.0;
            add_p += effect.1;
        }
        available.swap(selected, count - index - 1);
    }
    ((1.0 + add_v) * (1.0 + add_p)).clamp(1.0, 6.0).floor() as i32
}

fn elder_penalty_range(rank: i32) -> (f32, f32, f32) {
    if rank < 1 {
        (0.8, 0.5, 5.0)
    } else if rank.abs() <= 1 {
        (0.3, 0.2, 5.0)
    } else {
        (0.05, 0.2, 2.0)
    }
}

fn item_has_random_name(data: &GameData, name: &str) -> bool {
    data.special_names.contains_key(name) || data.art_names.contains_key(name)
}

/// Standard population by cultivation stage, from GameDefine.SchoolJianghuNpcCount.
pub const SECT_NPC_COUNTS: [i32; 6] = [15, 15, 20, 20, 15, 15];

pub fn extract_all_sect_npcs_with_settings(
    world_seed: i32,
    d: &GameData,
    settings: NpcGenerationSettings,
) -> Vec<SectData> {
    let fallback = [
        (1, "丹霞洞天"),
        (2, "昆仑宫"),
        (3, "极天宫"),
        (4, "紫霄宗"),
        (5, "玄一道"),
        (6, "青莲剑宗"),
        (7, "栖霞洞天"),
        (8, "百蛮山"),
        (9, "七仟坞"),
        (10, "七杀魔宫"),
        (11, "合欢派"),
        (12, "万妖殿"),
        (13, "武当派"),
    ];
    let mut schools: Vec<(i32, &str)> = if d.schools.is_empty() {
        fallback.into_iter().collect()
    } else {
        d.schools
            .iter()
            .filter(|s| s.id > 0 && s.id < 500)
            .map(|s| (s.id, s.name.as_str()))
            .collect()
    };
    schools.sort_by_key(|&(id, _)| id);
    schools.dedup_by_key(|s| s.0);
    schools
        .into_iter()
        .map(|(school, name)| {
            let mut seeds = Vec::new();
            // Elders first, then ascending ordinary cultivation stages.
            for level in [5usize, 0, 1, 2, 3, 4] {
                for index in 0..SECT_NPC_COUNTS[level] {
                    seeds.push((level as i32, index));
                }
                // Modded/named members may be assigned outside the default population.
                let mut additional: Vec<_> = d
                    .location_to_template
                    .keys()
                    .filter_map(|&seed| {
                        let seed_school = seed / 100_000;
                        let seed_level = (seed % 100_000) / 10_000;
                        let index = seed % 10_000;
                        (seed_school == school
                            && seed_level == level as i32
                            && index >= SECT_NPC_COUNTS[level])
                            .then_some((seed_level, index))
                    })
                    .collect();
                additional.sort_unstable();
                seeds.extend(additional);
            }
            SectData {
                sect_name: name.to_owned(),
                npcs: seeds
                    .into_iter()
                    .map(|(level, index)| {
                        extract_npc_with_settings(school, level, index, world_seed, d, settings)
                    })
                    .collect(),
            }
        })
        .collect()
}

fn random_name_component(names: &[String], rng: &mut GRandom) -> String {
    if names.is_empty() {
        return String::new();
    }
    names[rng.rand_range(1, names.len() as i32 + 1) as usize - 1].clone()
}

fn random_race<'a>(d: &'a GameData, rule: &'a str, rng: &mut GRandom) -> &'a str {
    if rule.is_empty() {
        return "Human";
    }
    match rule {
        "RANDOM" => {
            if rng.random_rate(0.6) || d.generated_races.is_empty() {
                "Human"
            } else {
                &d.generated_races[rng.rand_range(0, d.generated_races.len() as i32) as usize]
            }
        }
        "RANDOMYAOGUAI" => {
            let races: Vec<&str> = d
                .generated_races
                .iter()
                .map(String::as_str)
                .filter(|&r| r != "Human")
                .collect();
            if races.is_empty() {
                "Human"
            } else {
                races[rng.rand_range(0, races.len() as i32) as usize]
            }
        }
        race => race,
    }
}

fn modifier_fabao_effect(d: &GameData, name: &str, scale: f32) -> (f32, f32) {
    if let Some(def) = d.modifier_defs.get(name) {
        return def
            .properties
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case("NpcFight_FabaoNum"))
            .fold((0.0, 0.0), |(v, p), prop| {
                (v + prop.add_v * scale, p + prop.add_p * scale)
            });
    }
    // Reference defaults make partial/synthetic data usable without dropping
    // the permanent base modifier or the FabaoDisp contribution.
    let add_p = match name {
        "Modifier_SpNpc_BasePropertie" => 1.0,
        "Modifier_SpNpc_FabaoDisp" => 0.5,
        "Modifier_SpNpc_FabaoNum" => 1.5,
        _ => 0.0,
    };
    (0.0, add_p * scale)
}

fn random_spell_item(
    d: &GameData,
    spell: Option<&str>,
    fixed_level: Option<usize>,
    quality: Option<f32>,
    rng: &mut dyn RngState,
) -> Option<ItemData> {
    const PAPERS: [&str; 3] = ["Item_Spell", "Item_SpellLv2", "Item_SpellLv3"];
    let spell_name = spell.map_or_else(|| pull_random_spell(d, rng), str::to_owned);
    let quality = quality.unwrap_or_else(|| rng.next_flt(0.1, 0.7));
    // RandomSpellItem chooses an item even when a fixed level overwrites it.
    let random_level = rng.next_int(0, PAPERS.len() as i32) as usize;
    let paper = PAPERS[fixed_level.unwrap_or(random_level)];
    let mut item = random_item_by_defname(d, paper, None, 0, 12, -1.0, rng)?;
    if spell_name.is_empty() {
        let _ = rng.next_flt(0.1, 0.7);
        item.quality = 0.5;
    } else {
        item.name = d.translate(&spell_name);
        item.quality = quality;
    }
    Some(item)
}

/// `EquipItemMaybeJingguai` always draws its probability for a non-null item.
/// A successful, eligible fresh item additionally draws `TongLing` lifetime,
/// movement delay and type; these remain on the same System.Random channel.
fn advance_awaken_rng(item: &ItemData, rng: &mut dyn RngState) {
    if rng.random_rate(0.0001) && item.tongling_eligible {
        let _ = rng.next_int(1, 99999);
        let _ = rng.next_flt(150.0, 20.0);
        let _ = rng.next_int(1, 4);
    }
}

/// Data needed solely to reproduce the game RNG progression while generating
/// skills; the scanner does not need to allocate the full combat objects.
#[derive(Clone, Copy)]
struct GeneratedFightSkill {
    kind: i32,
    element: i32,
    shape: i32,
    radius: f32,
}

fn advance_fight_skill_rng(
    d: &GameData,
    rng: &mut dyn RngState,
    fight_mode: bool,
) -> GeneratedFightSkill {
    let kind = random_fight_skill_kind(rng, fight_mode);
    let element = rng.next_int(1, 6);
    let selection = rng.next_int(0, fight_skill_selection_count(kind));
    let self_target = kind != 1 && selection == 0;
    let thing_target = kind == 2 && selection == 1;
    let shape = random_fight_skill_shape(rng, self_target);
    advance_fight_skill_position_rng(shape, rng);
    let radius = random_fight_skill_radius(thing_target, rng);
    advance_fight_skill_effect_rng(d, kind, element, shape, radius, rng);
    for (min, max) in [(0.8, 1.2), (0.5, 1.5), (0.5, 1.5), (0.5, 1.5), (0.5, 1.5)] {
        let _ = rng.next_flt(min, max);
    }
    GeneratedFightSkill {
        kind,
        element,
        shape,
        radius,
    }
}

fn random_fight_skill_kind(rng: &mut dyn RngState, fight_mode: bool) -> i32 {
    let mut kind = rng.next_int(1, 6);
    if rng.random_rate(0.5) {
        kind = 1;
    }
    if fight_mode && kind == 4 {
        kind = 1;
    }
    kind
}

fn fight_skill_selection_count(kind: i32) -> i32 {
    match kind {
        1 => 3,
        2 => 5,
        _ => 2,
    }
}

fn random_fight_skill_shape(rng: &mut dyn RngState, self_target: bool) -> i32 {
    if !self_target && rng.random_rate(0.4) {
        1
    } else {
        rng.next_int(if self_target { 2 } else { 1 }, 3)
    }
}

fn advance_fight_skill_position_rng(shape: i32, rng: &mut dyn RngState) {
    if shape == 1 || shape == 4 {
        for (min, max) in [(-5.0, 5.0), (5.0, 10.0), (-5.0, 5.0), (-5.0, 5.0)] {
            if rng.random_rate(0.5) {
                let _ = rng.next_flt(min, max);
            }
        }
    }
}

fn random_fight_skill_radius(thing_target: bool, rng: &mut dyn RngState) -> f32 {
    let mut radius = 0.0;
    if thing_target {
        if rng.random_rate(0.3) {
            radius = rng.next_flt(1.0, 3.0) as i32 as f32;
        }
        if rng.random_rate(0.2) {
            radius += rng.next_flt(0.0, 2.0) as i32 as f32;
        }
    }
    radius
}

fn advance_fight_skill_effect_rng(
    d: &GameData,
    kind: i32,
    element: i32,
    shape: i32,
    radius: f32,
    rng: &mut dyn RngState,
) {
    let feel = match kind {
        1 => 1,
        2 | 3 | 5 => 2,
        _ => 0,
    };
    match shape {
        1 => {
            advance_effect_rng(d, 1, feel, element, rng);
            advance_effect_rng(d, 2, feel, element, rng);
            if radius > 0.0 {
                advance_effect_rng(d, 3, feel, element, rng);
            }
        }
        2 => advance_effect_rng(d, 2, feel, element, rng),
        3 => advance_effect_rng(d, 3, feel, element, rng),
        4 => advance_effect_rng(d, 4, feel, element, rng),
        _ => {}
    }
}

fn advance_effect_rng(
    d: &GameData,
    shape: i32,
    mut feel: i32,
    mut element: i32,
    rng: &mut dyn RngState,
) {
    if feel == 1 || (feel == 2 && rng.random_rate(0.3)) {
        feel = 0;
    }
    if element != 0 && rng.random_rate(0.3) {
        element = 0;
    }
    if !d
        .effect_pool_counts
        .keys()
        .any(|&(f, s, _)| f == feel && s == shape)
    {
        if feel != 0 {
            advance_effect_rng(d, shape, 0, 0, rng);
        }
        return;
    }
    if let Some(&count) = d.effect_pool_counts.get(&(feel, shape, element)) {
        let _ = rng.next_int(0, count);
    } else if element != 0 {
        advance_effect_rng(d, shape, feel, 0, rng);
    }
}

fn name_table_count(d: &GameData, name: &str, row: usize) -> i32 {
    d.auxiliary_tables
        .get(name)
        .and_then(|rows| rows.get(row.saturating_sub(1)))
        .and_then(|columns| columns.first())
        .map_or(1, |value| value.split(',').count() as i32)
}

fn advance_fight_skill_name_rng(d: &GameData, skill: GeneratedFightSkill, rng: &mut dyn RngState) {
    advance_fight_skill_prefix_rng(d, skill.element, rng);
    let row = fight_skill_mid_row(skill);
    let _ = rng.next_int(
        0,
        name_table_count(d, "Display/EsotericaName/SkillMid.txt", row),
    );
    if rng.random_rate(0.8) {
        let _ = rng.next_int(
            1,
            name_table_count(d, "Display/EsotericaName/SkillSuffix.txt", 1),
        );
    }
}

fn advance_fight_skill_prefix_rng(d: &GameData, element: i32, rng: &mut dyn RngState) {
    match rng.next_int(1, 4) {
        1 => {
            let _ = rng.next_int(
                1,
                name_table_count(
                    d,
                    "Display/EsotericaName/ElementPrefix.txt",
                    element as usize,
                ),
            );
        }
        2 => {
            let _ = rng.next_int(
                1,
                name_table_count(d, "Display/EsotericaName/GongPrefix.txt", 4),
            );
        }
        3 => advance_story_prefix_rng(d, rng),
        _ => {}
    }
}

fn advance_story_prefix_rng(d: &GameData, rng: &mut dyn RngState) {
    let _ = rng.next_int(
        1,
        name_table_count(d, "Display/EsotericaName/StoryPrefix.txt", 1),
    );
    let sex = rng.next_int(1, 3);
    let (prefixes, suffixes) = name_components(d, "Human", sex);
    for names in [prefixes, suffixes] {
        if !names.is_empty() {
            let _ = rng.next_int(1, names.len() as i32 + 1);
        }
    }
}

fn fight_skill_mid_row(skill: GeneratedFightSkill) -> usize {
    let base = match skill.kind {
        1 => 1,
        3 => 3,
        4 => 5,
        2 => 7,
        _ => 9,
    };
    let first_row = if skill.kind == 1 {
        skill.shape == 1
    } else {
        skill.radius == 0.0
    };
    base + usize::from(!first_row)
}
