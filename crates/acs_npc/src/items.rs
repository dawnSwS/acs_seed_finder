use super::{constants::FABAO_OITEM, data::GameData, rng_state::RngState, types::ItemData};

pub fn random_item_defname(
    d: &GameData,
    lable: &str,
    minr: i32,
    maxr: i32,
    rng: &mut dyn RngState,
) -> Option<String> {
    let list = d.map_lable2item.get(lable)?;
    if list.is_empty() {
        return None;
    }
    if minr <= 0 && maxr >= 12 {
        return Some(list[rng.next_int(0, list.len() as i32) as usize].clone());
    }
    let mut enumerator_idx = (rng.next_int(1, list.len() as i32 + 1) as usize - 1) % list.len();
    for _ in 0..list.len() {
        let current_name = &list[enumerator_idx];
        if let Some(def) = d.thing_defs.get(current_name) {
            if def.rate <= maxr && def.rate >= minr && def.not_random == 0 {
                return Some(current_name.clone());
            }
        }
        enumerator_idx = (enumerator_idx + 1) % list.len();
    }
    Some(list[rng.next_int(0, list.len() as i32) as usize].clone())
}

/// C# ThingMgr.cs:1717-1739 RandomClothDefname(label, season)
/// Season filter: Summer(1)->temp_max_add>=4.0, Winter(3)->temp_min_add<=-6.0, Else(0,2)->moderate
/// Empty filtered list falls back to random pick from original list (1 RNG consumed)
pub fn random_cloth_defname(
    d: &GameData,
    lable: &str,
    season: i32,
    rng: &mut dyn RngState,
) -> Option<String> {
    let list = d.map_lable2item.get(lable)?;
    if list.is_empty() {
        return None;
    }

    // Season-based filtering (C# ThingMgr.cs:1717-1739)
    let filtered: Vec<usize> = (0..list.len())
        .filter(|&i| {
            if let Some(def) = d.thing_defs.get(&list[i]) {
                if def.not_random > 0 || def.other_world > 0 {
                    return false;
                }
                match season {
                1 /* Summer */ => def.temp_max_add >= 4.0,
                3 /* Winter */ => def.temp_min_add <= -6.0,
                _ /* Else(Spring/Autumn) */ => def.temp_max_add < 4.0 && def.temp_min_add > -6.0,
            }
            } else {
                false
            }
        })
        .collect();

    // Fallback: if season filter yields empty list, random pick from full list (1 RNG)
    if filtered.is_empty() {
        return Some(list[rng.next_int(0, list.len() as i32) as usize].clone());
    }

    let mut enumerator_idx =
        (rng.next_int(1, filtered.len() as i32 + 1) as usize - 1) % filtered.len();
    for _ in 0..filtered.len() {
        let current_name = &list[filtered[enumerator_idx]];
        if let Some(def) = d.thing_defs.get(current_name) {
            if def.not_random <= 0 && def.other_world <= 0 {
                return Some(current_name.clone());
            }
        }
        enumerator_idx = (enumerator_idx + 1) % filtered.len();
    }
    None
}

pub fn random_stuff_item_defname(
    d: &GameData,
    category: &str,
    minrate: i32,
    maxrate: i32,
    rng: &mut dyn RngState,
) -> Option<String> {
    let list = d.map_lable2stuff.get(category)?;
    if list.is_empty() {
        return None;
    }
    if maxrate >= 12 && minrate <= 0 {
        return Some(list[rng.next_int(0, list.len() as i32) as usize].clone());
    }
    let mut enumerator_idx = (rng.next_int(1, list.len() as i32 + 1) as usize - 1) % list.len();
    for _ in 0..list.len() {
        let current_name = &list[enumerator_idx];
        if let Some(def) = d.thing_defs.get(current_name) {
            if def.rate <= maxrate && def.rate >= minrate {
                return Some(current_name.clone());
            }
        }
        enumerator_idx = (enumerator_idx + 1) % list.len();
    }
    Some(list[rng.next_int(0, list.len() as i32) as usize].clone())
}

pub fn random_item_by_defname(
    d: &GameData,
    itemname: &str,
    stuff: Option<&str>,
    minrate: i32,
    maxrate: i32,
    qadd: f32,
    rng: &mut dyn RngState,
) -> Option<ItemData> {
    // C# ItemRandomMachine.cs: def not found → return null, 0 RNG consumed
    let def = match d.thing_defs.get(itemname) {
        Some(d) => d,
        None => return None,
    };
    let mut display_name = d.translate(itemname);
    if def.has_be_made && !def.stuff_categories.is_empty() {
        let mut text = stuff.unwrap_or("").to_string();
        if text.is_empty() || text == "None" {
            let sc_idx = rng.next_int(0, def.stuff_categories.len() as i32) as usize;
            let sc_name = &def.stuff_categories[sc_idx];
            text = random_stuff_item_defname(d, sc_name, minrate, maxrate, rng)
                .unwrap_or_else(|| "Item_Wood".to_string());
        }
        if !text.is_empty() && text != "None" {
            display_name = format!("{}({})", display_name, d.translate(&text));
        }
        return Some(ItemData {
            name: display_name,
            quality: if qadd >= 0.0 {
                qadd
            } else {
                rng.next_flt(0.1, 0.5)
            },
        });
    }
    // C# ItemRandomMachine.cs:92-93 — non-BeMade also consumes 1 Float RNG for quality
    Some(ItemData {
        name: display_name,
        quality: if qadd >= 0.0 {
            qadd
        } else {
            rng.next_flt(0.1, 0.5)
        },
    })
}

pub fn random_fabao(
    d: &GameData,
    lable: &str,
    minr: i32,
    maxr: i32,
    q: f32,
    rng: &mut dyn RngState,
) -> Option<ItemData> {
    // C# ItemRandomMachine.cs: "if (rate != 0f) rate = (int)RandomRange(minr, maxr, t)"
    // rate parameter in C# maps to q (quality/rate float param) in Rust
    let rate = if q != 0.0 {
        rng.next_int(minr, maxr)
    } else {
        0
    };
    let f_idx = rng.next_int(0, FABAO_OITEM.len() as i32) as usize;
    let mut oitem_defname = random_item_defname(d, FABAO_OITEM[f_idx], 0, rate, rng);
    if oitem_defname.is_none() {
        oitem_defname = random_item_defname(d, "Other", 0, rate, rng);
    }
    let oitem_defname = oitem_defname?;

    let mut element_kind = 0;
    if let Some(def) = d.thing_defs.get(&oitem_defname) {
        element_kind = def.element_kind;
        if def.has_be_made && !def.stuff_categories.is_empty() {
            let s_idx = rng.next_int(0, def.stuff_categories.len() as i32) as usize;
            let stuff = random_stuff_item_defname(d, &def.stuff_categories[s_idx], 0, rate, rng)
                .unwrap_or_else(|| "Item_Wood".to_string());
            if let Some(s_def) = d.thing_defs.get(&stuff) {
                if s_def.element_kind != 0 {
                    element_kind = s_def.element_kind;
                }
            }
            // C# ItemRandomMachine.cs: oitem BeMade branch consumes Float RNG for quality
            let _ = rng.next_flt(0.1, 0.5);
        }
        // C# ItemRandomMachine.cs: non-BeMade and def-not-found do NOT consume Float RNG here
    }

    let is_god = rng.random_rate(0.3);
    let limit = if !is_god { 11 } else { 12 };
    let final_rate = rate.clamp(0, limit);
    let final_q = if q != 0.0 { q } else { rng.next_flt(0.1, 0.8) };

    for _ in 0..10 {
        let _ = rng.next_flt(0.75, 1.15);
    }

    if lable == "TreasureFabao" {
        let mut num_f = 0.5 + 0.15 * (final_rate as f32);
        if final_rate >= 12 {
            num_f += 0.75;
        }

        let mut num5 = 1;
        if rng.random_rate(final_q / 3.0 + num_f - 1.0) {
            num5 = 2;
        }
        for i in 0..num5 {
            if i != 0 {
                let _ = rng.next_int(0, 100);
            }
            let _ = rng.next_flt(0.7, 1.3);
        }
    }

    let mut row = 1;
    if element_kind != 0 && rng.random_rate(0.8) {
        row = element_kind + 1;
    }
    let _ = rng.next_int(1, *d.fabao_prefix_lengths.get(&row).unwrap_or(&1) + 1);
    let mid_k = rng.next_int(1, 4);
    let _ = rng.next_int(1, *d.fabao_mid_lengths.get(&mid_k).unwrap_or(&1) + 1);
    let suffix_len = d
        .thing_defs
        .get(&oitem_defname)
        .map_or(1, |x| x.fabao_suffix_len);
    let _ = rng.next_int(0, 1.max(suffix_len as i32));

    Some(ItemData {
        name: format!("法宝·{}", d.translate(&oitem_defname)),
        quality: final_q,
    })
}

pub fn random_item_by_lable(
    d: &GameData,
    lable: &str,
    minrate: i32,
    maxrate: i32,
    q: f32,
    stuff: Option<&str>,
    is_cloth: bool,
    rng: &mut dyn RngState,
) -> Option<ItemData> {
    if lable == "FightFabao" || lable == "TreasureFabao" {
        return random_fabao(d, lable, minrate, maxrate, q, rng);
    }
    let text = if is_cloth {
        random_cloth_defname(d, lable, 0, rng) // season=0 (Spring/Else, C# ThingMgr.cs:1717-1739)
    } else {
        random_item_defname(d, lable, minrate, maxrate, rng)
    }?;

    let def = match d.thing_defs.get(&text) {
        Some(d) => d,
        None => {
            return Some(ItemData {
                name: d.translate(&text),
                quality: if q == 0.0 { -1.0 } else { q },
            })
        }
    };

    let mut display_name = d.translate(&text);
    let mut quality = if q == 0.0 { -1.0 } else { q };

    if def.has_be_made && !def.stuff_categories.is_empty() {
        let mut s_text = stuff.unwrap_or("").to_string();
        if s_text.is_empty() || s_text == "None" {
            let sc_idx = rng.next_int(0, def.stuff_categories.len() as i32) as usize;
            let sc_name = &def.stuff_categories[sc_idx];
            s_text = random_stuff_item_defname(d, sc_name, minrate, maxrate, rng)
                .unwrap_or_else(|| "Item_Wood".to_string());
        }
        if !s_text.is_empty() && s_text != "None" {
            display_name = format!("{}({})", display_name, d.translate(&s_text));
        }
        if q == 0.0 {
            quality = rng.next_flt(0.1, 0.5);
        }
    } else {
        // 非制作品且没有直接指定品质时，直接返回默认值，绝不空转质量随机数
        if q == 0.0 {
            quality = 0.5;
        }
    }

    Some(ItemData {
        name: display_name,
        quality,
    })
}
