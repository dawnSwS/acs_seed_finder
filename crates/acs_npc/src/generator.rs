use crate::{
    constants::{get_rich_data, CLOTHES_STUFF, S_ITEM_LABELS},
    data::GameData,
    items::{
        random_item_by_defname, random_item_by_lable, random_item_defname,
        random_stuff_item_defname,
    },
    rng_state::RngState,
    types::{DropItemData, ItemData, NpcData, SectData},
};
use acs_core::rng::{DotNetRandom, GRandom};

#[inline]
pub fn pull_random_spell(d: &GameData, rng_sys: &mut dyn RngState) -> String {
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

pub fn get_jh_npc_local_seed(mut school: i32, rlevel: i32, index: i32) -> i32 {
    if school < 0 {
        school = 999;
    } else if school > 998 {
        school = 998;
    }
    (school * 100_000) + (rlevel * 10_000) + index
}

pub fn extract_npc(school: i32, rlevel: i32, index: i32, world_seed: i32, d: &GameData) -> NpcData {
    let local_seed = get_jh_npc_local_seed(school, rlevel, index);
    let num = world_seed.wrapping_add(local_seed);
    let mut rng_jh = GRandom::new(num as u32);
    // BUG1修复: 用location_to_template直接查找模板（复制C# GetJHNpcTemplateName逻辑）
    let jh_def_base = d
        .location_to_template
        .get(&local_seed)
        .and_then(|name| d.name_to_index.get(name))
        .map(|&i| &d.jh_npcs_global[i]);

    let mut sex = jh_def_base.map(|def| def.sex).unwrap_or(0);
    if sex <= 0 {
        sex = rng_jh.rand_range(1, 3);
    }

    let (mut last_name, mut first_name) = (
        jh_def_base
            .map(|def| def.last_name.clone())
            .unwrap_or_default(),
        jh_def_base
            .map(|def| def.first_name.clone())
            .unwrap_or_default(),
    );

    // BUG2修复: 恢复C#的条件逻辑（只有LastName或FristName为空时才随机取名）
    if last_name.is_empty() || first_name.is_empty() {
        // C#: World.SetRander(emJianghu, new GRandom((uint)num)); 重新初始化RNG状态
        rng_jh = GRandom::new(num as u32);
        let prefix_idx = if !d.names_prefix.is_empty() {
            rng_jh.rand_range(1, d.names_prefix.len() as i32 + 1) - 1
        } else {
            0
        };
        let suffix_pool = if sex == 1 {
            &d.names_m_suffix
        } else {
            &d.names_f_suffix
        };
        let suffix_idx = if !suffix_pool.is_empty() {
            rng_jh.rand_range(1, suffix_pool.len() as i32 + 1) - 1
        } else {
            0
        };
        if last_name.is_empty() {
            last_name = if !d.names_prefix.is_empty() {
                d.names_prefix[prefix_idx as usize].clone()
            } else {
                "無名".to_string()
            };
        }
        if first_name.is_empty() {
            first_name = if !suffix_pool.is_empty() {
                suffix_pool[suffix_idx as usize].clone()
            } else {
                "氏".to_string()
            };
        }
    }

    let mut npc_name = format!("{}{}", last_name, first_name);
    let mut gong_level = jh_def_base.map(|def| def.gong_level).unwrap_or(0);
    if gong_level == 0 {
        gong_level = match rlevel {
            1 => 1,
            2 => 2,
            3 => 3,
            4 => 4,
            5 => 5,
            _ => 0,
        };
    }

    let title_str = jh_def_base.map(|def| def.title.clone()).unwrap_or_default();
    if !title_str.is_empty() {
        let titles: Vec<&str> = title_str.split(',').collect();
        if !titles.is_empty() {
            rng_jh = GRandom::new(num as u32);
            npc_name = format!(
                "【{}】{}",
                titles[rng_jh.rand_range(0, titles.len() as i32) as usize].trim(),
                npc_name
            );
        }
    }

    rng_jh = GRandom::new(num as u32);
    let _feature = rng_jh.rand_range(1, 7);

    rng_jh = GRandom::new((num as u32).wrapping_add(1));
    for _ in 0..rng_jh.rand_range(1, 5) {
        let _ = random_item_defname(
            d,
            S_ITEM_LABELS[rng_jh.rand_range(0, 24) as usize],
            2,
            if gong_level <= 0 { 4 } else { 12 },
            &mut rng_jh,
        );
    }

    rng_jh = GRandom::new((num as u32).wrapping_add(2));
    for _ in 0..rng_jh.rand_range(1, 5) {
        let _ = random_item_defname(
            d,
            S_ITEM_LABELS[rng_jh.rand_range(0, 24) as usize],
            0,
            if gong_level <= 0 { 4 } else { 9 },
            &mut rng_jh,
        );
    }

    let mut carry_list = jh_def_base.map(|def| def.carry.clone()).unwrap_or_default();
    if carry_list.is_empty() {
        let mut carry_rng = GRandom::new((num as u32).wrapping_add(3));
        let num4 = carry_rng.rand_range(1, gong_level + 1);
        for _ in 0..num4 {
            let is_eso = if carry_rng.random_rate(0.3) && gong_level > 0 {
                1
            } else {
                0
            };
            if is_eso == 0 {
                if let Some(defname) = random_item_defname(
                    d,
                    S_ITEM_LABELS[carry_rng.rand_range(0, 24) as usize],
                    if gong_level <= 0 { 1 } else { 4 },
                    if gong_level <= 0 {
                        5
                    } else {
                        5.max(gong_level * 3)
                    },
                    &mut carry_rng,
                ) {
                    carry_list.push(DropItemData {
                        thing_type: 0,
                        thing_def: defname,
                        stuff: String::new(),
                    });
                }
            } else {
                carry_list.push(DropItemData {
                    thing_type: 1,
                    thing_def: String::new(),
                    stuff: String::new(),
                });
            }
        }
    }

    if gong_level > 0 {
        let mut rng_jh_eso = GRandom::new(num as u32);
        let _ = rng_jh_eso.rand_range(1, 8);
    }

    let mut rng_jh_hobby = GRandom::new(num as u32);
    // Note: Keep rand_range(0, 10). GameData lacks hobby data, but since rand() is called
    // exactly once per rand_range, the RNG state advancement is identical regardless of range arguments.
    for _ in 0..rng_jh_hobby.rand_range(1, 4) {
        let _ = rng_jh_hobby.rand_range(0, 10);
    }

    rng_jh = GRandom::new(num as u32);
    let _num3 = match rlevel {
        1 => rng_jh.rand_range(1, 4),
        2 => rng_jh.rand_range(4, 7),
        3 => rng_jh.rand_range(7, 10),
        4 => rng_jh.rand_range(10, 12),
        5 => 12,
        _ => 0,
    };
    let lable_idx = rng_jh.rand_range(0, 5);

    let (mut talismans, mut inventory) = (Vec::new(), Vec::new());
    let mut rng_sys = DotNetRandom::new(num);
    let gongkind_dao = school != 12;

    // Note: This God2 equipment path has been compared with C# SpNpcMgr source but may still have
    // discrepancies in RNG channel routing:
    // - C# uses gRandom[emNone] (GRandom after SetRandomSeed) for equipment via World.RandomRange
    // - Rust uses a separate DotNetRandom (rng_sys) for equipment
    // - MakeGold uses PRE-SetRandomSeed state in C# but a fresh GRandom in Rust
    if rlevel == 5 {
        let mut temp_rng = DotNetRandom::new(num);
        let num_mods = if gongkind_dao { 9 } else { 6 };
        let mut jumpmap: Vec<usize> = (0..num_mods).collect();
        let (mut picked_fabao_num, mut fabao_num_scale) = (false, 0.0);

        for j in 0..(if gongkind_dao { 4 } else { 3 }) {
            let num2 = temp_rng.next_int(0, (num_mods - j) as i32) as usize;
            // 无条件推进一次随机数以实现与 C# 端同步
            let scale_val = if j != 0 {
                temp_rng.next_int(100, 201)
            } else {
                temp_rng.next_int(300, 501)
            } as f32
                / 100.0;
            if gongkind_dao && jumpmap[num2] == 6 {
                picked_fabao_num = true;
                fabao_num_scale = scale_val;
            }
            jumpmap.swap(num2, num_mods - j - 1);
        }

        let mut fabao_num_add_p = 0.0;
        let mut fabao_num_add_v = 0.0;
        if picked_fabao_num {
            if let Some(mdef) = d.modifier_defs.get("Modifier_SpNpc_FabaoNum") {
                for prop in &mdef.properties {
                    if prop.name.eq_ignore_ascii_case("NpcFight_FabaoNum") {
                        fabao_num_add_v += prop.add_v * fabao_num_scale;
                        fabao_num_add_p += prop.add_p * fabao_num_scale;
                    }
                }
            } else {
                fabao_num_add_p += 1.5 * fabao_num_scale;
            }
        }

        let fabao_count = ((1.0 + fabao_num_add_v) * (1.0 + fabao_num_add_p))
            .clamp(1.0, 6.0)
            .floor() as i32;

        rng_jh = GRandom::new(num as u32);
        rng_sys = DotNetRandom::new(num);

        if gongkind_dao {
            let _ = rng_jh.rand_range(250_000, 750_000);
            let num_skills = rng_jh.rand_range(2, 6);
            // C# SpNpcMgr.cs:362 — skill loop uses emNone (System.Random/DotNetRandom)
            // Note: FightSkillMgr.SkillTemplate.RandomNewDef and RandomName use emJianghu (GRAND)
            // but those do not affect equipment/talismans RNG sequences; omitted as approximation.
            for _ in 0..num_skills {
                let _ = rng_sys.next_int(0, 8);
            }

            let array = get_rich_data(4);
            let stuff = CLOTHES_STUFF[rng_sys.next_int(0, CLOTHES_STUFF.len() as i32) as usize];
            if let Some(item) = random_item_by_lable(
                d,
                "Clothes",
                11,
                11,
                rng_sys.next_flt(array.0, array.1),
                Some(stuff),
                true,
                &mut rng_sys,
            ) {
                inventory.push(item);
                rng_sys.random_rate(0.0001);
            }
            if let Some(item) = random_item_by_lable(
                d,
                "Trousers",
                11,
                11,
                rng_sys.next_flt(array.0, array.1),
                Some(stuff),
                true,
                &mut rng_sys,
            ) {
                inventory.push(item);
                rng_sys.random_rate(0.0001);
            }
            if let Some(item) = random_item_by_lable(
                d,
                "Weapon",
                11,
                11,
                rng_sys.next_flt(array.0, array.1),
                None,
                false,
                &mut rng_sys,
            ) {
                inventory.push(item);
                rng_sys.random_rate(0.0001);
            }

            for _ in 0..fabao_count {
                if let Some(item) =
                    random_item_by_lable(d, "FightFabao", 12, 12, 0.0, None, false, &mut rng_sys)
                {
                    inventory.push(item);
                    rng_sys.random_rate(0.0001);
                }
            }

            for _ in 0..3 {
                let spell_name = pull_random_spell(d, &mut rng_sys);
                let _ = rng_sys.next_int(0, 3);
                let _ = random_item_by_defname(d, "Item_SpellLv3", None, 0, 12, -1.0, &mut rng_sys);
                talismans.push(ItemData {
                    name: if spell_name.is_empty() {
                        d.translate("Item_SpellLv3")
                    } else {
                        format!("神符·{}", d.translate(&spell_name))
                    },
                    quality: 1.0,
                });
            }
        } else {
            // 体修大能: _EquipSpNpc 路径
            let array = get_rich_data(4);
            let stuff = CLOTHES_STUFF[rng_sys.next_int(0, CLOTHES_STUFF.len() as i32) as usize];
            if let Some(item) = random_item_by_lable(
                d,
                "Clothes",
                11,
                11,
                rng_sys.next_flt(array.0, array.1),
                Some(stuff),
                true,
                &mut rng_sys,
            ) {
                inventory.push(item);
                rng_sys.random_rate(0.0001);
            }
            if let Some(item) = random_item_by_lable(
                d,
                "Trousers",
                11,
                11,
                rng_sys.next_flt(array.0, array.1),
                Some(stuff),
                true,
                &mut rng_sys,
            ) {
                inventory.push(item);
                rng_sys.random_rate(0.0001);
            }
            if let Some(item) = random_item_by_lable(
                d,
                "Weapon",
                11,
                11,
                rng_sys.next_flt(array.0, array.1),
                None,
                false,
                &mut rng_sys,
            ) {
                inventory.push(item);
                rng_sys.random_rate(0.0001);
            }
            // 体修不生成 FightFabao（由 gongkind == Body 控制）
        }
    } else {
        let r_data = get_rich_data(lable_idx);
        let (minr, maxr) = (r_data.2, r_data.3);

        // 1. 衣服 (noClothes条件: glevel==0 && school==11)
        let no_clothes = _num3 == 0 && school == 11;
        if !no_clothes && (sex == 2 || rng_sys.random_rate(0.8)) {
            if let Some(item) = random_item_by_lable(
                d,
                "Clothes",
                minr,
                maxr,
                rng_sys.next_flt(r_data.0, r_data.1),
                None,
                true,
                &mut rng_sys,
            ) {
                inventory.push(item);
                rng_sys.random_rate(0.0001);
            }
        }
        // 2. 裤子 (100%)
        if let Some(item) = random_item_by_lable(
            d,
            "Trousers",
            minr,
            maxr,
            rng_sys.next_flt(r_data.0, r_data.1),
            None,
            true,
            &mut rng_sys,
        ) {
            inventory.push(item);
            rng_sys.random_rate(0.0001);
        }
        // 3. 武器 (lable <= Normal → 50%)
        let weapon_rate = if lable_idx <= 2 { 0.5 } else { 1.0 };
        if rng_sys.random_rate(weapon_rate) {
            if let Some(item) = random_item_by_lable(
                d,
                "Weapon",
                minr,
                maxr,
                rng_sys.next_flt(r_data.0, r_data.1),
                None,
                false,
                &mut rng_sys,
            ) {
                inventory.push(item);
                rng_sys.random_rate(0.0001);
            }
        }
        // 4. 法宝 (道修 0.1%)
        if gongkind_dao && rng_sys.random_rate(0.001) {
            if let Some(item) = random_item_by_lable(
                d,
                "TreasureFabao",
                minr,
                maxr,
                rng_sys.next_flt(r_data.0, r_data.1),
                None,
                false,
                &mut rng_sys,
            ) {
                inventory.push(item);
                rng_sys.random_rate(0.0001);
            }
        }
        // 5. 工具 (Rich+ 10%)
        if lable_idx >= 3 && rng_sys.random_rate(0.1) {
            if let Some(item) =
                random_item_by_lable(d, "Tool", 0, 5, 0.0, None, false, &mut rng_sys)
            {
                inventory.push(item);
                rng_sys.random_rate(0.0001);
            }
        }
        // 6. 符箓 (道修 + Rich+ 1%)
        if gongkind_dao && lable_idx >= 3 && rng_sys.random_rate(0.01) {
            let spell_name = pull_random_spell(d, &mut rng_sys);
            let q_fu = rng_sys.next_flt(0.1, 0.7);
            let _ = rng_sys.next_int(0, 3);
            let _ = rng_sys.next_flt(0.1, 0.5);
            talismans.push(ItemData {
                name: if spell_name.is_empty() {
                    d.translate("Item_SpellPaperLv1")
                } else {
                    format!("神符·{}", d.translate(&spell_name))
                },
                quality: q_fu,
            });
            rng_sys.random_rate(0.0001);
        }

        // 7. glevel > 0 时的额外生成
        if _num3 > 0 && gongkind_dao {
            // 技能点消耗 (简化: 按等级范围消耗RNG)
            let skill_pts = match rlevel {
                1 => (0, 5),
                2 => (5, 20),
                3 => (20, 50),
                4 | 5 => (30, 75),
                _ => (0, 0),
            };
            if skill_pts.1 > 0 {
                let pts = rng_sys.next_int(skill_pts.0, skill_pts.1);
                for _ in 0..pts {
                    let _ = rng_sys.next_int(0, 10);
                }
            }

            let _ = rng_sys.next_int(0, 10);
            if rlevel >= 3 {
                let _ = rng_sys.next_int(1000, 20_000);
                while rng_sys.random_rate(0.4) {
                    let _ = rng_sys.next_int(1000, 20_000);
                }
            }
            if rlevel < 3 {
                let _ = rng_sys.next_flt(0.1, 1.0);
            }

            // 8. 秘籍 (30%)
            if rng_sys.random_rate(0.3) {
                inventory.push(ItemData {
                    name: "门派无名秘籍".into(),
                    quality: 1.0,
                });
                rng_sys.random_rate(0.0001);
            }
            // 9. 符箓 (20%)
            if rng_sys.random_rate(0.2) {
                let spell_name = pull_random_spell(d, &mut rng_sys);
                let q_fu = rng_sys.next_flt(0.1, 0.7);
                let _ = rng_sys.next_int(0, 3);
                let _ = rng_sys.next_flt(0.1, 0.5);
                talismans.push(ItemData {
                    name: if spell_name.is_empty() {
                        d.translate("Item_SpellPaperLv1")
                    } else {
                        format!("神符·{}", d.translate(&spell_name))
                    },
                    quality: q_fu,
                });
                rng_sys.random_rate(0.0001);
            }
            // 10. 战斗力参数
            let _ = rng_sys.next_flt(1.0, 3.0);
            let _ = rng_sys.next_flt(0.75, 1.0);
            // 11. 战斗法宝 (按 NpcFight_FabaoNum)
            if let Some(item) =
                random_item_by_lable(d, "FightFabao", 1, _num3, 0.0, None, false, &mut rng_sys)
            {
                inventory.push(item);
                rng_sys.random_rate(0.0001);
            }
        }
    }

    if school == 11 {
        // 使用独立的 emJianghu RNG 生成品质，避免污染主序列
        let mut rng_11 = GRandom::new(num as u32);
        talismans.push(ItemData {
            name: d.translate("Spell_TempLow1"),
            quality: rng_11.rand_float(0.6, 0.8),
        });
        talismans.push(ItemData {
            name: d.translate("Spell_TempHigh1"),
            quality: rng_11.rand_float(0.6, 0.8),
        });
    }

    for (i, carry) in carry_list.iter().enumerate() {
        let mut item_rng = GRandom::new((num as u32).wrapping_add(i as u32));
        if carry.thing_type == 0 {
            let mut display_name = d.translate(&carry.thing_def);
            if let Some(def) = d.thing_defs.get(&carry.thing_def) {
                if def.has_be_made && !def.stuff_categories.is_empty() {
                    let mut text =
                        if carry.stuff.is_empty() || carry.stuff.eq_ignore_ascii_case("None") {
                            ""
                        } else {
                            carry.stuff.as_str()
                        }
                        .to_string();
                    if text.is_empty() {
                        text = random_stuff_item_defname(
                            d,
                            &def.stuff_categories
                                [item_rng.next_int(0, def.stuff_categories.len() as i32) as usize],
                            0,
                            12,
                            &mut item_rng,
                        )
                        .unwrap_or_else(|| "Item_Wood".to_string());
                    }
                    display_name = format!("{}({})", display_name, d.translate(&text));
                }
            }
            let item = ItemData {
                name: display_name.clone(),
                quality: item_rng.next_flt(0.1, 0.5),
            };
            if carry.thing_def.to_lowercase().starts_with("spell_") || display_name.contains("符")
            {
                talismans.push(item);
            } else {
                inventory.push(item);
            }
        } else if carry.thing_type == 1 {
            inventory.push(ItemData {
                name: d.translate("未知秘录 (随机推演掉落)"),
                quality: 0.0,
            });
        }
    }

    NpcData {
        name: npc_name,
        talismans,
        inventory,
    }
}

pub fn extract_all_sect_npcs(world_seed: i32, d: &GameData) -> Vec<SectData> {
    [
        (1, "丹霞洞天"),
        (2, "昆仑宫"),
        (3, "栖天宫"),
        (4, "紫霄宫"),
        (5, "玄一道"),
        (6, "青莲剑宗"),
        (7, "栖霞洞天"),
        (8, "百蛮山"),
        (9, "七仟坞"),
        (10, "七杀魔宫"),
        (11, "合欢派"),
        (12, "万妖殿"),
        (13, "武当派"),
    ]
    .iter()
    .map(|&(sect_id, sect_name)| {
        let mut npcs = Vec::new();
        for i in 0..13 {
            npcs.push(extract_npc(sect_id, 5, i, world_seed, d));
        }
        for &level in &[0, 1, 2, 3, 4] {
            for i in 0..5 {
                npcs.push(extract_npc(sect_id, level, i, world_seed, d));
            }
        }
        SectData {
            sect_name: sect_name.to_string(),
            npcs,
        }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::GameData;

    /// Helper: create an empty GameData (all maps/lists are default/empty).
    /// extract_npc uses it for random names/items — all lookups return empty/None,
    /// so the function still produces deterministic output.
    fn empty_game_data() -> GameData {
        GameData::default()
    }

    /// C1 – Idempotency: same (world_seed, school, level, index) → same output
    #[test]
    fn extract_npc_idempotent() {
        let d = empty_game_data();
        let params = [
            (0, 1, 1, 1),
            (110001, 0, 0, 0),
            (42, 12, 5, 10),
            (99999, 3, 2, 0),
        ];
        for (world, school, level, idx) in params {
            let a = extract_npc(school, level, idx, world, &d);
            let b = extract_npc(school, level, idx, world, &d);
            assert_eq!(
                a.name, b.name,
                "name mismatch for ({},{},{},{})",
                world, school, level, idx
            );
            assert_eq!(
                a.talismans.len(),
                b.talismans.len(),
                "talismans len mismatch for ({},{},{},{})",
                world,
                school,
                level,
                idx
            );
            assert_eq!(
                a.inventory.len(),
                b.inventory.len(),
                "inventory len mismatch for ({},{},{},{})",
                world,
                school,
                level,
                idx
            );
            assert_eq!(
                a.talismans, b.talismans,
                "talismans mismatch for ({},{},{},{})",
                world, school, level, idx
            );
            assert_eq!(
                a.inventory, b.inventory,
                "inventory mismatch for ({},{},{},{})",
                world, school, level, idx
            );
        }
    }

    /// C2 – num formula equivalence
    /// Same `num` with same `rlevel` and compatible `school` → identical output.
    ///
    /// The original spec params (world=0, school=1, level=1, index=1) vs (world=110001, school=0, level=0, index=0)
    /// share num=110001 but differ in `rlevel` (1 vs 0), which causes different `gong_level`,
    /// `_num3`, and RNG consumption paths — outputs necessarily diverge.
    ///
    /// This test uses the same `rlevel` (0) and schools that don't affect branching (both != 11, != 12):
    ///   A: (school=0, rlevel=0, index=0, world=200000) → local=0,      num=200000
    ///   B: (school=1, rlevel=0, index=0, world=100000) → local=100000, num=200000
    #[test]
    fn extract_npc_num_formula_equivalence() {
        let d = empty_game_data();
        let npc_a = extract_npc(0, 0, 0, 200_000, &d); // local=0,      num=200000
        let npc_b = extract_npc(1, 0, 0, 100_000, &d); // local=100000, num=200000
        assert_eq!(
            npc_a.name, npc_b.name,
            "num formula: names should match when num is equal (same rlevel)"
        );
        assert_eq!(
            npc_a.talismans.len(),
            npc_b.talismans.len(),
            "num formula: talismans count should match"
        );
        assert_eq!(
            npc_a.inventory.len(),
            npc_b.inventory.len(),
            "num formula: inventory count should match"
        );
        assert_eq!(
            npc_a.talismans, npc_b.talismans,
            "num formula: talismans should match"
        );
        assert_eq!(
            npc_a.inventory, npc_b.inventory,
            "num formula: inventory should match"
        );
    }

    /// C3 – smoke: a matrix of (seed, school 1..3, level 0..5, index 0..2)
    /// must not panic
    #[test]
    fn extract_npc_smoke_matrix() {
        let d = empty_game_data();
        for world_seed in [0i32, 42, 110001, 999999] {
            for school in 1..=3 {
                for level in 0..=5 {
                    for idx in 0..=2 {
                        let npc = extract_npc(school, level, idx, world_seed, &d);
                        // basic sanity: name is non-empty
                        assert!(
                            !npc.name.is_empty(),
                            "empty name at seed={}, school={}, level={}, idx={}",
                            world_seed,
                            school,
                            level,
                            idx
                        );
                    }
                }
            }
        }
    }

    /// C4 – get_jh_npc_local_seed clamp behaviour
    #[test]
    fn local_seed_clamp() {
        // school < 0 → clamped to 999
        assert_eq!(get_jh_npc_local_seed(-1, 0, 0), 999 * 100_000);
        // school > 998 → clamped to 998
        assert_eq!(get_jh_npc_local_seed(999, 0, 0), 998 * 100_000);
        // normal case
        assert_eq!(get_jh_npc_local_seed(5, 3, 7), 5 * 100_000 + 3 * 10_000 + 7);
    }
}
