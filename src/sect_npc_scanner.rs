use crate::rng::DotNetRandom;
use std::{collections::HashMap, fs, path::{Path, PathBuf}};

struct CsDict<K, V> { keys: Vec<K>, pub values: Vec<V>, indices: HashMap<K, usize> }
impl<K: Eq + std::hash::Hash + Clone, V> CsDict<K, V> {
    fn new() -> Self { Self { keys: vec![], values: vec![], indices: HashMap::new() } }
    fn insert(&mut self, k: K, v: V) {
        if let Some(&i) = self.indices.get(&k) { self.values[i] = v; }
        else { self.indices.insert(k.clone(), self.keys.len()); self.keys.push(k); self.values.push(v); }
    }
}

#[derive(Default)]
pub struct GameData { pub clothes: Vec<String>, pub pants: Vec<String>, pub weapons: Vec<String>, pub stuffs: Vec<String>, pub spells: Vec<String>, pub loaded: bool }

impl GameData {
    pub fn new() -> Self { Self::default() }
    pub fn load_from_dir(base: &Path) -> Self {
        let (mut item_dict, mut spell_dict, mut files) = (CsDict::new(), CsDict::new(), vec![]);
        fn walk(d: &Path, f: &mut Vec<PathBuf>) {
            if let Ok(es) = fs::read_dir(d) {
                for p in es.flatten().map(|e| e.path()) {
                    if p.is_dir() { walk(&p, f); } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml")) { f.push(p); }
                }
            }
        }
        walk(&base.join("ThingDef"), &mut files); walk(&base.join("SpellDef"), &mut files); files.sort();

        let txt = |b: &str, t: &str| b.split_once(&format!("<{t}>"))?.1.split_once(&format!("</{t}>")).map(|(s, _)| s.trim().to_string());
        for p in files {
            if let Ok(c) = fs::read_to_string(&p) {
                for b in c.split("</ThingDef>").filter_map(|s| s.split_once("<ThingDef").map(|(_, b)| b)) {
                    let n = b.split_once("Name=\"").and_then(|(_, r)| r.split_once('"')).map(|(s, _)| s.to_string()).or_else(|| txt(b, "defName")).unwrap_or_default();
                    if n.is_empty() { continue; }
                    let (tn, it, et) = (txt(b, "ThingName").unwrap_or_else(|| n.clone()), txt(b, "ItemType").unwrap_or_default(), txt(b, "EquipType").unwrap_or_default());
                    item_dict.insert(n, (tn, it.clone(), et, txt(b, "IsStuff").is_some_and(|s| s.eq_ignore_ascii_case("true") || s == "1") || it == "Material" || it == "Stuff" || b.contains("<StuffDef>")));
                }
                for b in c.split("</SpellDef>").filter_map(|s| s.split_once("<SpellDef").map(|(_, b)| b)) {
                    let n = b.split_once("Name=\"").and_then(|(_, r)| r.split_once('"')).map(|(s, _)| s.to_string()).or_else(|| txt(b, "defName")).unwrap_or_default();
                    if !n.is_empty() { spell_dict.insert(n.clone(), txt(b, "SpellName").or_else(|| txt(b, "ThingName")).unwrap_or(n)); }
                }
            }
        }
        let mut d = Self::default();
        for (tn, it, et, is_stuff) in item_dict.values {
            if et == "Clothes" { d.clothes.push(tn.clone()); }
            if et == "Pants" || et == "Trousers" { d.pants.push(tn.clone()); }
            if et == "Weapon" || it == "Fabao" { d.weapons.push(tn.clone()); }
            if is_stuff { d.stuffs.push(tn); }
        }
        d.spells = spell_dict.values; d.loaded = !d.clothes.is_empty(); d
    }
}

#[derive(Debug, Clone)] pub struct AbstractLoot { pub category: String, pub item_name: String, pub stuff_name: String, pub quality: f32 }
#[derive(Debug, Clone)] pub struct NpcInventory { pub seed: i32, pub loots: Vec<AbstractLoot>, pub wealth: i32, pub has_jackpot: bool, pub best_fu_quality: f32 }

pub fn exhaust_sect_elder_inventory(rng: &mut DotNetRandom, glevel: i32, vip: bool, d: &GameData) -> NpcInventory {
    let mut inv = NpcInventory { seed: 0, loots: vec![], wealth: 0, has_jackpot: false, best_fu_quality: 0.0 };
    let get_cnt = |l: usize, def| 1.max(if d.loaded { l as i32 } else { def });
    let (cc, pc, wc, sc, stc) = (get_cnt(d.clothes.len(), 45), get_cnt(d.pants.len(), 45), get_cnt(d.weapons.len(), 45), get_cnt(d.spells.len(), 50), get_cnt(d.stuffs.len(), 80));

    for (p, pool, list) in [("衣服 (上衣)", cc, &d.clothes), ("裤子 (下装)", pc, &d.pants), ("本命法宝", wc, &d.weapons)] {
        let (i_idx, _, s_idx, q) = (rng.next_range_strict(0, pool), rng.next_range_strict(0, 10), rng.next_range_strict(0, stc), rng.next_float(0.1, 0.5));
        let get_name = |loaded, l: &Vec<String>, idx, def_fmt: &str| if loaded { l.get(idx as usize).cloned().unwrap_or_default() } else { format!("{def_fmt}_#{idx}") };
        inv.loots.push(AbstractLoot { category: p.into(), item_name: get_name(d.loaded, list, i_idx, p), stuff_name: get_name(d.loaded, &d.stuffs, s_idx, "材质"), quality: q });
    }

    if glevel > 0 {
        rng.next_range_strict(0, 12);
        if glevel >= 6 { inv.wealth = rng.next_range_strict(1000, 20000) + std::iter::from_fn(|| rng.random_rate(if vip { 0.8 } else { 0.4 }).then(|| rng.next_range_strict(1000, 20000))).sum::<i32>(); }
        if vip || rng.random_rate(0.3) { rng.box_muller_trap(); }
        if rng.random_rate(if vip { 0.6 } else { 0.2 }) {
            let (s_idx, fq) = (rng.next_range_strict(0, sc), rng.next_float(0.1, 0.7));
            inv.has_jackpot = fq > 0.690; inv.best_fu_quality = fq;
            inv.loots.push(AbstractLoot { category: "神符 (Talisman)".into(), item_name: if d.loaded { d.spells.get(s_idx as usize).cloned().unwrap_or_default() } else { format!("神符_#{s_idx}") }, stuff_name: "天道造化".into(), quality: fq });
        }
    } inv
}