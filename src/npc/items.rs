// This port preserves the C# API's integer indices and f32 arithmetic.
// Validated game pools are small; indices come from the bounded RngState contract.
#![allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::indexing_slicing
)]

use super::{
    constants::FABAO_OITEM,
    data::GameData,
    rng_state::RngState,
    types::{ItemData, ThingDefData},
};

pub fn random_item_defname(
    d: &GameData,
    label: &str,
    min_rate: i32,
    max_rate: i32,
    rng: &mut dyn RngState,
) -> Option<String> {
    select_definition(
        d,
        d.map_lable2item.get(label)?,
        min_rate,
        max_rate,
        false,
        rng,
    )
}

fn select_definition(
    d: &GameData,
    pool: &[String],
    min_rate: i32,
    max_rate: i32,
    allow_not_random: bool,
    rng: &mut dyn RngState,
) -> Option<String> {
    if pool.is_empty() {
        return None;
    }
    if min_rate <= 0 && max_rate >= 12 {
        return Some(pool[rng.next_int(0, pool.len() as i32) as usize].clone());
    }
    let start = (rng.next_int(1, pool.len() as i32 + 1) as usize - 1) % pool.len();
    for offset in 0..pool.len() {
        let name = &pool[(start + offset) % pool.len()];
        let eligible = d.thing_defs.get(name).is_some_and(|def| {
            def.rate <= max_rate
                && def.rate >= min_rate
                && (allow_not_random || def.not_random == 0)
        });
        if eligible {
            return Some(name.clone());
        }
    }
    Some(pool[rng.next_int(0, pool.len() as i32) as usize].clone())
}

/// Select a start in the complete pool, then walk to the first seasonal match.
pub fn random_cloth_defname(
    d: &GameData,
    label: &str,
    season: i32,
    rng: &mut dyn RngState,
) -> Option<String> {
    let pool = d.map_lable2item.get(label)?;
    if pool.is_empty() {
        return None;
    }
    let start = (rng.next_int(1, pool.len() as i32 + 1) - 1) as usize;
    for offset in 0..pool.len() {
        let name = &pool[(start + offset) % pool.len()];
        if d.thing_defs
            .get(name)
            .is_some_and(|def| suitable_clothing(def, season))
        {
            return Some(name.clone());
        }
    }
    None
}

fn suitable_clothing(def: &ThingDefData, season: i32) -> bool {
    if def.not_random > 0 || def.other_world > 0 {
        return false;
    }
    match season {
        2 => def.temp_max_add >= 2.0,
        4 => def.temp_min_add <= -4.0,
        _ => def.temp_max_add < 4.0 && def.temp_min_add > -6.0,
    }
}

fn materialized_name(
    d: &GameData,
    item: &str,
    stuff: Option<&str>,
    rng: &mut dyn RngState,
) -> String {
    let base = d.translate(item);
    let Some(stuff) = stuff else {
        return base;
    };
    if let Some(names) = d
        .special_names
        .get(item)
        .and_then(|by_stuff| by_stuff.get(stuff))
        .filter(|names| !names.is_empty())
    {
        return names[rng.next_int(0, names.len() as i32) as usize].clone();
    }
    if d.thing_defs
        .get(item)
        .is_some_and(|def| def.no_material_prefix)
    {
        return base;
    }
    d.thing_defs
        .get(stuff)
        .filter(|def| def.is_stuff)
        .map_or(base.clone(), |def| {
            format!("{}{base}", def.material_name_prefix)
        })
}

fn finish_item(
    d: &GameData,
    item: &str,
    name: String,
    quality: f32,
    rng: &mut dyn RngState,
) -> ItemData {
    let has_quality = d.thing_defs.get(item).is_some_and(|def| {
        [
            "Weapon",
            "FightFabao",
            "TreasureFabao",
            "Hat",
            "Clothes",
            "Trousers",
            "Tool",
            "Spell",
        ]
        .contains(&def.label.as_str())
    });
    let quality = if has_quality { quality } else { -1.0 };
    let name = if quality >= 0.9 {
        d.art_names
            .get(item)
            .filter(|names| !names.is_empty())
            .map_or(name, |names| {
                names[rng.next_int(0, names.len() as i32) as usize].clone()
            })
    } else {
        name
    };
    ItemData {
        name,
        quality,
        tongling_eligible: d
            .thing_defs
            .get(item)
            .is_some_and(|def| def.tongling_eligible),
    }
}

pub fn random_stuff_item_defname(
    d: &GameData,
    category: &str,
    min_rate: i32,
    max_rate: i32,
    rng: &mut dyn RngState,
) -> Option<String> {
    select_definition(
        d,
        d.map_lable2stuff.get(category)?,
        min_rate,
        max_rate,
        true,
        rng,
    )
}

fn manufacturing_stuff(
    d: &GameData,
    def: &ThingDefData,
    stuff: Option<&str>,
    min_rate: i32,
    max_rate: i32,
    rng: &mut dyn RngState,
) -> String {
    let text = stuff.unwrap_or_default();
    if !text.is_empty() && text != "None" {
        return text.to_owned();
    }
    let category =
        &def.stuff_categories[rng.next_int(0, def.stuff_categories.len() as i32) as usize];
    random_stuff_item_defname(d, category, min_rate, max_rate, rng)
        .unwrap_or_else(|| "Item_Wood".to_owned())
}

fn generated_quality(quality: f32, rng: &mut dyn RngState) -> f32 {
    if quality >= 0.0 {
        quality
    } else {
        rng.next_flt(0.1, 0.5)
    }
}

fn material_quality(d: &GameData, stuff: &str, quality: f32) -> f32 {
    (quality + d.thing_defs.get(stuff).map_or(0.0, |def| def.made_quality)).clamp(0.0, 1.0)
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
    random_item_by_defname_inner(d, itemname, stuff, minrate, maxrate, qadd, rng, None)
}

#[allow(clippy::too_many_arguments)]
pub fn random_item_by_defname_with_naming_rng(
    d: &GameData,
    itemname: &str,
    stuff: Option<&str>,
    minrate: i32,
    maxrate: i32,
    qadd: f32,
    rng: &mut dyn RngState,
    naming_rng: &mut dyn RngState,
) -> Option<ItemData> {
    random_item_by_defname_inner(
        d,
        itemname,
        stuff,
        minrate,
        maxrate,
        qadd,
        rng,
        Some(naming_rng),
    )
}

#[allow(clippy::too_many_arguments)]
fn random_item_by_defname_inner(
    d: &GameData,
    item_name: &str,
    stuff: Option<&str>,
    min_rate: i32,
    max_rate: i32,
    quality: f32,
    rng: &mut dyn RngState,
    naming_rng: Option<&mut dyn RngState>,
) -> Option<ItemData> {
    let def = d.thing_defs.get(item_name)?;
    let stuff = (def.has_be_made && !def.stuff_categories.is_empty())
        .then(|| manufacturing_stuff(d, def, stuff, min_rate, max_rate, rng));
    // Defname generation draws quality even for nonmanufactured items.
    let quality = generated_quality(quality, rng);
    let quality = stuff
        .as_deref()
        .map_or(quality, |stuff| material_quality(d, stuff, quality));
    let stuff = stuff
        .as_deref()
        .filter(|text| !text.is_empty() && *text != "None");
    Some(match naming_rng {
        Some(naming) => named_item(d, item_name, stuff, quality, naming),
        None => named_item(d, item_name, stuff, quality, rng),
    })
}

fn named_item(
    d: &GameData,
    item: &str,
    stuff: Option<&str>,
    quality: f32,
    rng: &mut dyn RngState,
) -> ItemData {
    let name = materialized_name(d, item, stuff, rng);
    finish_item(d, item, name, quality, rng)
}

pub fn random_fabao(
    d: &GameData,
    label: &str,
    min_rate: i32,
    max_rate: i32,
    quality: f32,
    rng: &mut dyn RngState,
) -> Option<ItemData> {
    // Rate and quality are independent parameters; rate always draws first.
    let rate = rng.next_int(min_rate, max_rate);
    let original = fabao_original(d, rate, rng)?;
    let element_kind = fabao_original_element(d, &original, rate, rng);
    let limit = if rng.random_rate(0.3) { 12 } else { 11 };
    let rate = rate.clamp(0, limit);
    let quality = if quality == 0.0 {
        rng.next_flt(0.1, 0.8)
    } else {
        quality
    };
    for _ in 0..if label == "FightFabao" { 10 } else { 1 } {
        let _ = rng.next_flt(0.75, 1.15);
    }
    match label {
        "TreasureFabao" => draw_treasure_properties(rate, quality, rng),
        "FightFabao" => draw_fight_properties(rate, element_kind, rng),
        _ => {}
    }
    Some(ItemData {
        name: random_fabao_name(d, &original, element_kind, rng),
        quality,
        tongling_eligible: d
            .thing_defs
            .get(if label == "FightFabao" {
                "FightFaBao"
            } else {
                "TreasureFabao"
            })
            .is_some_and(|def| def.tongling_eligible),
    })
}

fn fabao_original(d: &GameData, rate: i32, rng: &mut dyn RngState) -> Option<String> {
    let label = FABAO_OITEM[rng.next_int(0, FABAO_OITEM.len() as i32) as usize];
    random_item_defname(d, label, 0, rate, rng)
        .or_else(|| random_item_defname(d, "Other", 0, rate, rng))
}

fn fabao_original_element(d: &GameData, original: &str, rate: i32, rng: &mut dyn RngState) -> i32 {
    let Some(def) = d.thing_defs.get(original) else {
        return 0;
    };
    let mut element = def.element_kind;
    if def.has_be_made && !def.stuff_categories.is_empty() {
        let stuff = manufacturing_stuff(d, def, None, 0, rate, rng);
        if let Some(stuff) = d.thing_defs.get(&stuff).filter(|def| def.element_kind != 0) {
            element = stuff.element_kind;
        }
        let _ = rng.next_flt(0.1, 0.5);
        let _ = materialized_name(d, original, Some(&stuff), rng);
    }
    element
}

fn draw_treasure_properties(rate: i32, quality: f32, rng: &mut dyn RngState) {
    let mut rate_weight = 0.5 + 0.15 * (rate as f32);
    if rate >= 12 {
        rate_weight += 0.75;
    }
    let count = 1 + i32::from(rng.random_rate(quality / 3.0 + rate_weight - 1.0));
    for index in 0..count {
        if index == 0 {
            // The game's loop condition draws emNone on every check.
            let mut skipped = 0;
            while skipped < rng.next_int(0, 3) {
                skipped += 1;
            }
        } else {
            let _ = rng.next_int(0, 100);
        }
        let _ = rng.next_flt(0.7, 1.3);
    }
}

fn draw_fight_properties(rate: i32, element: i32, rng: &mut dyn RngState) {
    if rate < 8 || element == 0 {
        let _ = rng.next_int(6, 9);
        if element != 0 {
            let _ = rng.random_rate(0.8);
        }
    } else {
        let _ = rng.next_int(1, 5);
    }
}

fn random_fabao_name(d: &GameData, original: &str, element: i32, rng: &mut dyn RngState) -> String {
    let row = if element != 0 && rng.random_rate(0.8) {
        element + 1
    } else {
        1
    };
    let prefix = rng.next_int(1, *d.fabao_prefix_lengths.get(&row).unwrap_or(&1));
    let mid_row = rng.next_int(1, 4);
    let mid = rng.next_int(1, *d.fabao_mid_lengths.get(&mid_row).unwrap_or(&1));
    let suffix_len = d
        .thing_defs
        .get(original)
        .map_or(1, |def| def.fabao_suffix_len);
    let suffix = rng.next_int(0, 1.max(suffix_len as i32));
    fabao_name(d, original, row, prefix, mid_row, mid, suffix)
}

fn fabao_name(
    d: &GameData,
    oitem_defname: &str,
    row: i32,
    prefix_index: i32,
    mid_k: i32,
    mid_index: i32,
    suffix_index: i32,
) -> String {
    let word = |path: &str, row: i32, column: i32| {
        d.auxiliary_tables
            .get(path)
            .and_then(|rows| rows.get((row - 1) as usize))
            .and_then(|row| row.first())
            .and_then(|cell| cell.split(',').nth(column as usize))
            .unwrap_or_default()
            .to_owned()
    };
    let prefix = word("Display/FabaoName/Prefix.txt", row, prefix_index);
    let mid = word("Display/FabaoName/Mid.txt", mid_k, mid_index);
    let suffix = d
        .thing_defs
        .get(oitem_defname)
        .and_then(|def| def.fabao_suffix.split(',').nth(suffix_index as usize))
        .unwrap_or_default()
        .trim();
    if prefix.is_empty() && mid.is_empty() && suffix.is_empty() {
        format!("法宝·{}", d.translate(oitem_defname))
    } else {
        format!(
            "{}{}{suffix}",
            prefix,
            if mid == prefix { "" } else { mid.as_str() }
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub fn random_item_by_lable_with_season(
    d: &GameData,
    lable: &str,
    minrate: i32,
    maxrate: i32,
    q: f32,
    stuff: Option<&str>,
    season: i32,
    rng: &mut dyn RngState,
) -> Option<ItemData> {
    if lable == "FightFabao" || lable == "TreasureFabao" {
        return random_fabao(d, lable, minrate, maxrate, q, rng);
    }
    let text = if season != 0 && (lable == "Clothes" || lable == "Trousers") {
        random_cloth_defname(d, lable, season, rng)
    } else {
        random_item_defname(d, lable, minrate, maxrate, rng)
    }?;

    let def = d.thing_defs.get(&text)?;
    let (name, quality) = label_item_properties(d, def, &text, stuff, minrate, maxrate, q, rng);
    Some(finish_item(d, &text, name, quality, rng))
}

#[allow(clippy::too_many_arguments)]
fn label_item_properties(
    d: &GameData,
    def: &ThingDefData,
    item: &str,
    stuff: Option<&str>,
    min_rate: i32,
    max_rate: i32,
    quality: f32,
    rng: &mut dyn RngState,
) -> (String, f32) {
    let name = d.translate(item);
    if !def.has_be_made || def.stuff_categories.is_empty() {
        // Label generation keeps default quality without an extra random draw.
        return (name, 0.0);
    }
    let material = manufacturing_stuff(d, def, stuff, min_rate, max_rate, rng);
    let quality = if quality == 0.0 {
        rng.next_flt(0.1, 0.5)
    } else {
        quality
    };
    let quality = material_quality(d, &material, quality);
    let name = if !material.is_empty() && material != "None" {
        materialized_name(d, item, Some(&material), rng)
    } else {
        name
    };
    (name, quality)
}
