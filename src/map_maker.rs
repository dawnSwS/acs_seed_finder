use crate::rng::{GMathUtl, RandomType};
use crate::terrain::Terrain;
use std::sync::OnceLock;

static BASE_AROUND_50: OnceLock<Vec<(i32, i32)>> = OnceLock::new();

fn get_base_around_50() -> &'static Vec<(i32, i32)> {
    BASE_AROUND_50.get_or_init(|| {
        let mut list = Vec::with_capacity(10201);
        list.push((0, 0));
        let mut num = 1; let mut i = 2; let mut dir = 1; let mut x = 0; let mut y = 0;
        let mut j = 1;
        while j < 10201 {
            let mut current_i = i;
            while current_i > 0 {
                for _ in 0..num {
                    match dir { 0 => x += 1, 1 => y -= 1, 2 => x -= 1, 3 => y += 1, _ => {} }
                    list.push((x, y)); j += 1;
                    if j >= 10201 { return list; }
                }
                dir = (dir + 1) % 4; current_i -= 1;
            }
            num += 1; i = 2;
        }
        list
    })
}

const T_SOIL: usize = 1;
const T_DEPTH_WATER: usize = 2;
const T_D_DEPTH_WATER: usize = 3;
const T_SHALLOW_WATER: usize = 4;
const T_MUD: usize = 5;
const T_FERTILE_SOIL: usize = 6;
const T_LING_SOIL: usize = 7;
const T_STONE_LAND: usize = 9;
const T_ROCK_BROWN: usize = 10;
const T_ROCK_GRAY: usize = 11;
const T_ROCK_MARBLE: usize = 12;
const T_IRON_ORE: usize = 13;
const T_COPPER_ORE: usize = 14;
const T_SILVER_ORE: usize = 15;

#[inline(always)]
fn get_t_idx(t: Terrain) -> usize {
    match t {
        Terrain::Soil => T_SOIL, Terrain::DepthWater => T_DEPTH_WATER, Terrain::DDepthWater => T_D_DEPTH_WATER,
        Terrain::ShallowWater => T_SHALLOW_WATER, Terrain::Mud => T_MUD, Terrain::FertileSoil => T_FERTILE_SOIL,
        Terrain::LingSoil => T_LING_SOIL, Terrain::StoneLand => T_STONE_LAND, Terrain::RockBrown => T_ROCK_BROWN,
        Terrain::RockGray => T_ROCK_GRAY, Terrain::RockMarble => T_ROCK_MARBLE, Terrain::IronOre => T_IRON_ORE,
        Terrain::CopperOre => T_COPPER_ORE, Terrain::SilverOre => T_SILVER_ORE, _ => 0,
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum CType { AllTrue, NoBorn, CheckCon, CheckCon2 }

pub struct MapMaker {
    pub width: i32,
    pub height: i32,
    pub grid: Vec<Terrain>,
    pub rand: GMathUtl,
    pub m_lis_mine_dir: Vec<i32>,
    pub born_space: Vec<bool>,
    pub cache_list: Vec<i32>,
    pub cache_born_line: Vec<bool>,
    
    pub bb: Vec<Vec<u32>>, 
}

impl MapMaker {
    pub fn new(seed: i32, width: i32, height: i32) -> Self {
        let grid_count = (width * height) as usize;
        let stride = ((width + 31) / 32) as usize;
        let bb_size = stride * height as usize;
        
        Self {
            width, height,
            rand: GMathUtl::new(seed),
            grid: vec![Terrain::Null; grid_count],
            m_lis_mine_dir: Vec::with_capacity(2048),
            born_space: vec![false; grid_count],
            cache_list: Vec::with_capacity(grid_count),
            cache_born_line: vec![false; grid_count],
            bb: vec![vec![0; bb_size]; 20],
        }
    }

    pub fn reset(&mut self, seed: i32) {
        self.rand = GMathUtl::new(seed);
        self.m_lis_mine_dir.clear();
        for b in &mut self.bb { b.fill(0); }
        self.grid.fill(Terrain::Null);
        self.born_space.fill(false);
        self.cache_born_line.fill(false);
    }

    #[inline(always)]
    pub fn p2key_safe(&self, x: i32, y: i32) -> i32 {
        if x < 0 || x >= self.width || y < 0 || y >= self.height { return -1; }
        y * self.width + x
    }

    #[inline(always)]
    pub fn key2p(&self, key: i32) -> (i32, i32) { (key % self.width, key / self.width) }

    #[inline(always)]
    pub fn is_valid_key(&self, key: i32) -> bool { key > 0 && key < self.width * self.height }

    #[inline(always)]
    fn set_mask(&mut self, t_target: usize, word: usize, mask: u32) {
        for t in 1..=15 {
            if t == t_target { self.bb[t][word] |= mask; } 
            else { self.bb[t][word] &= !mask; }
        }
    }

    #[inline(always)]
    fn get_ctype_mask(&self, w: usize, ctype: CType) -> u32 {
        let mut base = match ctype {
            CType::AllTrue => 0xFFFFFFFF,
            CType::NoBorn => {
                let fs = self.bb[T_SOIL][w] | self.bb[T_FERTILE_SOIL][w];
                let bp = self.bb[16][w] | self.bb[17][w];
                fs & !bp
            },
            CType::CheckCon => {
                self.bb[T_SOIL][w] | self.bb[T_FERTILE_SOIL][w] | self.bb[T_LING_SOIL][w]
            },
            CType::CheckCon2 => {
                let bad = self.bb[T_IRON_ORE][w] | self.bb[T_COPPER_ORE][w] | self.bb[T_SILVER_ORE][w] 
                        | self.bb[T_ROCK_BROWN][w] | self.bb[T_ROCK_GRAY][w];
                let marble = self.bb[T_ROCK_MARBLE][w];
                let bp = self.bb[16][w] | self.bb[17][w];
                !(bad | (marble & !bp))
            }
        };
        let stride = ((self.width + 31) / 32) as usize;
        let rem = self.width % 32;
        if rem != 0 && (w % stride) == stride - 1 { base &= (1 << rem) - 1; }
        base
    }

    #[inline(always)]
    pub fn get_grid(&self, key: i32, dir: u8) -> i32 {
        if key == -1 { return -1; }
        let size = self.width;
        let grid_count = size * self.height;
        match dir {
            0 => { let num = key + size; if num > 0 && num < grid_count { num } else { -1 } }
            1 => { let num = key - size; if num > 0 && num < grid_count { num } else { -1 } }
            2 => { let num = key - 1; if num >= 0 && num < grid_count && (key / size) == (num / size) { num } else { -1 } }
            3 => { let num = key + 1; if num >= 0 && num < grid_count && (key / size) == (num / size) { num } else { -1 } }
            4 => { let num = self.get_grid(key, 2); if num != -1 { self.get_grid(num, 1) } else { -1 } }
            5 => { let num = self.get_grid(key, 3); if num != -1 { self.get_grid(num, 1) } else { -1 } }
            6 => { let num = self.get_grid(key, 2); if num != -1 { self.get_grid(num, 0) } else { -1 } }
            7 => { let num = self.get_grid(key, 3); if num != -1 { self.get_grid(num, 0) } else { -1 } }
            _ => -1,
        }
    }

    #[inline(always)]
    fn get_cpu_neighbor(&self, key: i32, out_keys: &mut [i32; 8]) -> usize {
        let mut count = 0;
        let dirs = [6, 4, 7, 5, 1, 2, 3, 0];
        for &dir in &dirs {
            let n = self.get_grid(key, dir);
            if n != -1 { out_keys[count] = n; count += 1; }
        }
        count
    }

    fn fill(&mut self, def: Terrain) {
        let def_u = get_t_idx(def);
        let stride = ((self.width + 31) / 32) as usize;
        let rem = self.width % 32;
        for t in 1..=15 {
            if t == def_u {
                for w in 0..self.bb[t].len() {
                    let xw = w % stride;
                    if rem != 0 && xw == stride - 1 { self.bb[t][w] = (1 << rem) - 1; } 
                    else { self.bb[t][w] = 0xFFFFFFFF; }
                }
            } else {
                self.bb[t].fill(0);
            }
        }
    }

    fn born_space_random_fill(&mut self, def: Terrain) -> i32 {
        let x = self.rand.random_range_int(self.width / 10 * 4, self.width / 10 * 7, RandomType::EmNone, "MakeMap");
        let y = self.rand.random_range_int(self.height / 10 * 6, self.height / 10 * 7, RandomType::EmNone, "MakeMap");
        let key = self.p2key_safe(x, y);
        if key >= 0 && key < self.width * self.height {
            let stride = ((self.width + 31) / 32) as usize;
            let nw = (y as usize) * stride + (x as usize) / 32;
            self.set_mask(get_t_idx(def), nw, 1 << (x % 32));
        }
        key
    }

    fn make_mine_dir(&mut self, fx: i32, fy: i32) {
        let mut i = 0;
        let mut num = self.rand.random_range_int(0, self.height, RandomType::EmNone, "MakeMineDir");
        let num2 = num; let _ = self.rand.random_range_int(0, self.height, RandomType::EmNone, "MakeMineDir");
        while i < self.width {
            if self.rand.random_range_int(0, 100, RandomType::EmNone, "MakeMineDir") < 10 { i -= 1; } else { i += 1; }
            if num2 > self.height / 2 { num += self.rand.random_range_int(-fy, fx, RandomType::EmNone, "MakeMineDir"); }
            else { num += self.rand.random_range_int(-fx, fy, RandomType::EmNone, "MakeMineDir"); }
            let key = self.p2key_safe(i, num);
            if self.is_valid_key(key) { self.m_lis_mine_dir.push(key); }
        }
    }

    fn random_line_from_mine_dir(&mut self, w: i32, size: i32, def: Terrain, ctype: CType) {
        if self.m_lis_mine_dir.is_empty() { return; }
        let def_u = get_t_idx(def);
        let stride = ((self.width + 31) / 32) as usize;
        let mut num = self.rand.random_range_int(0, self.m_lis_mine_dir.len() as i32, RandomType::EmNone, "MakeMap") as usize;
        
        for _ in 0..size {
            if num >= self.m_lis_mine_dir.len() { num = self.rand.random_range_int(0, self.m_lis_mine_dir.len() as i32, RandomType::EmNone, "MakeMap") as usize; }
            let mut key = self.m_lis_mine_dir[num]; num += 1;
            
            let mut v_keys = [0i32; 8];
            let n_count = self.get_cpu_neighbor(key, &mut v_keys);
            if n_count > 0 { key = v_keys[self.rand.random_range_int(0, n_count as i32, RandomType::EmNone, "MakeMap") as usize]; }
            
            if key >= 0 && key < self.width * self.height {
                let key_x = key % self.width; let key_y = key / self.width;
                let nw = (key_y as usize) * stride + (key_x as usize) / 32;
                let nbit = 1 << (key_x % 32);
                if (self.get_ctype_mask(nw, ctype) & nbit) != 0 { self.set_mask(def_u, nw, nbit); }
            }
        }
        self.out_line(def, def, w, 4, 0, ctype);
    }

    fn out_line(&mut self, src: Terrain, t_target: Terrain, w: i32, lv: i32, maxcount: i32, ctype: CType) {
        let src_u = get_t_idx(src); let target_u = get_t_idx(t_target);
        self.bb[18].copy_from_slice(&self.bb[src_u]);
        
        let limit = ((w * 2 + 1) * (w * 2 + 1)) as usize;
        let base_around_50 = get_base_around_50();
        let mut current_max = maxcount;
        let stride = ((self.width + 31) / 32) as usize;
        let total_words = stride * self.height as usize;
        
        for word in (0..total_words).rev() {
            let mut mask = self.bb[18][word];
            if word == 0 { mask &= !1; }
            while mask != 0 {
                let bit = 31 - mask.leading_zeros(); mask ^= 1 << bit;
                let cx = ((word % stride) * 32 + bit as usize) as i32;
                let cy = (word / stride) as i32;
                if cx >= self.width { continue; }
                
                for i in 0..limit {
                    if i >= base_around_50.len() { break; }
                    let (dx, dy) = base_around_50[i];
                    let nx = cx + dx; let ny = cy + dy;
                    if nx >= 0 && nx < self.width && ny >= 0 && ny < self.height {
                        if nx == 0 && ny == 0 { continue; }
                        if self.rand.random_range_int(0, 100, RandomType::EmNone, "OutLine") > lv { continue; }
                        let nw = (ny as usize) * stride + (nx as usize) / 32;
                        let n_bit = 1 << (nx % 32);
                        if (self.get_ctype_mask(nw, ctype) & n_bit) != 0 {
                            self.set_mask(target_u, nw, n_bit);
                            current_max -= 1;
                            if current_max == 0 { return; } 
                        }
                    }
                }
            }
        }
    }

    fn optimize(&mut self, def: Terrain, opt_min: i32, opt_max: i32, maxcount: i32, ctype: CType) {
        let def_u = get_t_idx(def);
        let stride = ((self.width + 31) / 32) as usize;
        let height = self.height as usize;
        let total_words = stride * height;
        
        for _ in 0..maxcount {
            for w in 0..total_words { self.bb[19][w] = self.get_ctype_mask(w, ctype); }
            
            for y in 0..height {
                let row_offset = y * stride;
                let up_offset = if y > 0 { (y - 1) * stride } else { row_offset };
                let dn_offset = if y < height - 1 { (y + 1) * stride } else { row_offset };
                
                for xw in 0..stride {
                    let w = row_offset + xw;
                    let valid_mask = self.bb[19][w];
                    if valid_mask == 0 { self.bb[18][w] = 0; continue; }
                    
                    let fetch_row = |offset: usize, is_up: bool| -> (u32, u32, u32) {
                        let mut c = self.bb[def_u][offset + xw];
                        if is_up && y == 1 && xw == 0 { c &= 0xFFFFFFFE; }
                        let c_prev = if xw > 0 { self.bb[def_u][offset + xw - 1] } else { 0 };
                        let c_next = if xw < stride - 1 { self.bb[def_u][offset + xw + 1] } else { 0 };
                        let l = (c << 1) | (c_prev >> 31);
                        let r = (c >> 1) | (c_next << 31);
                        (l, c, r)
                    };
                    
                    let (mid_l, _, mid_r) = fetch_row(row_offset, false);
                    let (up_l, up_c, up_r) = if y > 0 { fetch_row(up_offset, true) } else { (0, 0, 0) };
                    let (dn_l, dn_c, dn_r) = if y < height - 1 { fetch_row(dn_offset, false) } else { (0, 0, 0) };
                    
                    macro_rules! ha { ($a:expr, $b:expr) => { ($a ^ $b, $a & $b) } }
                    macro_rules! fa { ($a:expr, $b:expr, $c:expr) => { { let t = $a ^ $b; (t ^ $c, ($a & $b) | (t & $c)) } } }
                    
                    let (s0_0, c1_0) = ha!(mid_l, mid_r);
                    let (s0_1, c1_1) = fa!(up_l, up_c, up_r);
                    let (s0_2, c1_2) = fa!(dn_l, dn_c, dn_r);

                    let (s0_01, c1_01) = fa!(s0_0, s0_1, s0_2);
                    let (s1_0, c2_0) = fa!(c1_0, c1_1, c1_2);
                    let (s1_1, c2_1) = ha!(s1_0, c1_01);
                    let (s2_0, c3_0) = ha!(c2_0, c2_1);
                    
                    let (bit0, bit1, bit2, bit3) = (s0_01, s1_1, s2_0, c3_0);
                    
                    let match_mask = if opt_min == 2 && opt_max == 4 {
                        (!bit3 & !bit2 & bit1) | (!bit3 & bit2 & !bit1 & !bit0)
                    } else if opt_min == 2 && opt_max == 6 {
                        (!bit3 & !bit2 & bit1) | (!bit3 & bit2 & !bit1) | (!bit3 & bit2 & bit1 & !bit0)
                    } else if opt_min == 2 && (opt_max == 8 || opt_max == 9) {
                        bit3 | bit2 | bit1
                    } else {
                        let mut m = 0;
                        for bit in 0..32 {
                            let count = ((bit3 >> bit) & 1) * 8 + ((bit2 >> bit) & 1) * 4 + ((bit1 >> bit) & 1) * 2 + ((bit0 >> bit) & 1);
                            if count >= opt_min as u32 && count <= opt_max as u32 { m |= 1 << bit; }
                        } m
                    };
                    self.bb[18][w] = match_mask & valid_mask;
                }
            }
            for w in 0..total_words {
                let diff = self.bb[18][w] & !self.bb[def_u][w];
                if diff != 0 { self.set_mask(def_u, w, diff); }
            }
        }
    }

    fn random_and_expand(&mut self, def: Terrain, rcount: i32, ecount: i32, expand_lv: i32, opt_lv: i32, opt_count: i32, ctype: CType, ectype: CType) {
        let def_u = get_t_idx(def);
        let rc = std::cmp::max(1, rcount);
        let stride = ((self.width + 31) / 32) as usize;
        let total_words = stride * self.height as usize;
        
        for _ in 0..rc {
            let x = self.rand.random_range_int(0, self.width, RandomType::EmNone, "MakeMap");
            let y = self.rand.random_range_int(0, self.height, RandomType::EmNone, "MakeMap");
            let key = self.p2key_safe(x, y);
            if key >= 0 && key < self.width * self.height {
                let nw = (y as usize) * stride + (x as usize) / 32;
                let nbit = 1 << (x % 32);
                if (self.get_ctype_mask(nw, ctype) & nbit) != 0 { self.set_mask(def_u, nw, nbit); }
            }
        }
        
        let mut flag = true;
        for _ in 0..ecount {
            if flag {
                for w in 0..total_words {
                    let mut mask = self.bb[def_u][w];
                    while mask != 0 {
                        let b = mask.trailing_zeros(); mask ^= 1 << b;
                        let cx = (w % stride * 32 + b as usize) as i32;
                        if cx < self.width {
                            let key = (w / stride) as i32 * self.width + cx;
                            if self.rand.random_range_int(0, 100, RandomType::EmNone, "MakeMap") <= expand_lv {
                                let mut n_keys = [0i32; 8];
                                for d in 0..self.get_cpu_neighbor(key, &mut n_keys) {
                                    let nx = (n_keys[d] % self.width) as usize; let ny = (n_keys[d] / self.width) as usize;
                                    let nw = ny * stride + nx / 32; let n_bit = 1 << (nx % 32);
                                    if (self.get_ctype_mask(nw, ectype) & n_bit) != 0 { self.set_mask(def_u, nw, n_bit); }
                                }
                            }
                        }
                    }
                }
            } else {
                for w in (0..total_words).rev() {
                    let mut mask = self.bb[def_u][w];
                    while mask != 0 {
                        let b = 31 - mask.leading_zeros(); mask ^= 1 << b;
                        let cx = (w % stride * 32 + b as usize) as i32;
                        if cx < self.width {
                            let key = (w / stride) as i32 * self.width + cx;
                            if self.rand.random_range_int(0, 100, RandomType::EmNone, "MakeMap") <= expand_lv {
                                let mut n_keys = [0i32; 8];
                                for d in 0..self.get_cpu_neighbor(key, &mut n_keys) {
                                    let nx = (n_keys[d] % self.width) as usize; let ny = (n_keys[d] / self.width) as usize;
                                    let nw = ny * stride + nx / 32; let n_bit = 1 << (nx % 32);
                                    if (self.get_ctype_mask(nw, ectype) & n_bit) != 0 { self.set_mask(def_u, nw, n_bit); }
                                }
                            }
                        }
                    }
                }
            }
            flag = !flag;
        }
        if opt_lv > 0 { self.optimize(def, opt_lv, opt_count, 1, ectype); }
    }

    fn finalize_grid(&mut self) {
        let stride = ((self.width + 31) / 32) as usize;
        self.grid.fill(Terrain::Null);
        self.born_space.fill(false);
        self.cache_born_line.fill(false);

        for y in 0..(self.height as usize) {
            for x in 0..(self.width as usize) {
                let key = y * (self.width as usize) + x;
                let w = y * stride + x / 32;
                let bit = 1 << (x % 32);
                
                if (self.bb[T_LING_SOIL][w] & bit) != 0 { self.grid[key] = Terrain::LingSoil; continue; }
                if (self.bb[T_STONE_LAND][w] & bit) != 0 { self.grid[key] = Terrain::StoneLand; continue; }
                if (self.bb[T_ROCK_BROWN][w] & bit) != 0 { self.grid[key] = Terrain::RockBrown; continue; }
                if (self.bb[T_ROCK_GRAY][w] & bit) != 0 { self.grid[key] = Terrain::RockGray; continue; }
                if (self.bb[T_ROCK_MARBLE][w] & bit) != 0 { self.grid[key] = Terrain::RockMarble; continue; }
                if (self.bb[T_IRON_ORE][w] & bit) != 0 { self.grid[key] = Terrain::IronOre; continue; }
                if (self.bb[T_COPPER_ORE][w] & bit) != 0 { self.grid[key] = Terrain::CopperOre; continue; }
                if (self.bb[T_SILVER_ORE][w] & bit) != 0 { self.grid[key] = Terrain::SilverOre; continue; }
                if (self.bb[T_MUD][w] & bit) != 0 { self.grid[key] = Terrain::Mud; continue; }
                if (self.bb[T_SHALLOW_WATER][w] & bit) != 0 { self.grid[key] = Terrain::ShallowWater; continue; }
                if (self.bb[T_DEPTH_WATER][w] & bit) != 0 { self.grid[key] = Terrain::DepthWater; continue; }
                if (self.bb[T_D_DEPTH_WATER][w] & bit) != 0 { self.grid[key] = Terrain::DDepthWater; continue; }
                if (self.bb[T_FERTILE_SOIL][w] & bit) != 0 { self.grid[key] = Terrain::FertileSoil; continue; }
                if (self.bb[T_SOIL][w] & bit) != 0 { self.grid[key] = Terrain::Soil; continue; }
            }
        }
        
        for w in 0..(stride * self.height as usize) {
            let mut m = self.bb[16][w];
            while m != 0 {
                let b = m.trailing_zeros(); m ^= 1 << b;
                let cx = w % stride * 32 + b as usize;
                if cx < self.width as usize { self.born_space[w / stride * self.width as usize + cx] = true; }
            }
            let mut m_line = self.bb[17][w];
            while m_line != 0 {
                let b = m_line.trailing_zeros(); m_line ^= 1 << b;
                let cx = w % stride * 32 + b as usize;
                if cx < self.width as usize { self.cache_born_line[w / stride * self.width as usize + cx] = true; }
            }
        }
    }

    pub fn make_map(&mut self) {
        let size_scale = std::cmp::max(1, self.width / 64);
        let stride = ((self.width + 31) / 32) as usize;
        let total_words = stride * self.height as usize;
        
        self.make_mine_dir(2, 3);
        self.make_mine_dir(2, 3);
        self.make_mine_dir(2, 3);
        
        self.fill(Terrain::Soil);
        let _borncenter = self.born_space_random_fill(Terrain::FertileSoil);
        self.out_line(Terrain::FertileSoil, Terrain::FertileSoil, 2, 100, 0, CType::AllTrue);
        self.out_line(Terrain::FertileSoil, Terrain::FertileSoil, 5, 20, 0, CType::AllTrue);
        self.optimize(Terrain::FertileSoil, 2, 4, 1, CType::AllTrue);
        
        for w in 0..total_words {
            let mut m = self.bb[T_FERTILE_SOIL][w];
            if w == 0 { m &= !1; }
            self.bb[16][w] = m;
        }
        
        self.out_line(Terrain::FertileSoil, Terrain::FertileSoil, 2, 100, 0, CType::AllTrue);
        
        for w in 0..total_words {
            let mut m = self.bb[T_FERTILE_SOIL][w];
            if w == 0 { m &= !1; }
            if m != 0 { self.set_mask(T_SOIL, w, m); }
        }
        
        self.bb[17].fill(0);
        let width_f = self.width as f32;
        for i in 0..4 {
            let num2 = self.rand.random_range_int((width_f * 0.3) as i32, (width_f * 0.6) as i32, RandomType::EmNone, "MakeMap");
            let mut j = 0;
            while j < self.rand.random_range_int(5, 15, RandomType::EmNone, "MakeMap") {
                let k = match i {
                    0 => self.p2key_safe(0, num2 + j),
                    1 => self.p2key_safe(self.width - 1, num2 + j),
                    2 => self.p2key_safe(num2 + j, 0),
                    _ => self.p2key_safe(num2 + j, self.width - 1),
                };
                if k >= 0 {
                    let nw = (k / self.width as i32 as usize) * stride + (k % self.width as i32 as usize) / 32;
                    self.bb[17][nw] |= 1 << (k % 32);
                } j += 1;
            }
        }
        
        self.random_and_expand(Terrain::FertileSoil, 20, 4, 30, 5, 3, CType::AllTrue, CType::AllTrue);
        self.random_and_expand(Terrain::DDepthWater, size_scale - 1, 2 * size_scale - 1, 13 + 6 * size_scale, 5, 3, CType::NoBorn, CType::NoBorn);
        self.out_line(Terrain::DDepthWater, Terrain::DepthWater, 1, 100, 0, CType::CheckCon);
        self.out_line(Terrain::DepthWater, Terrain::DepthWater, 1, 10 + 6 * size_scale, 0, CType::NoBorn);
        self.out_line(Terrain::DepthWater, Terrain::ShallowWater, 4, 50 + 12 * size_scale, 0, CType::CheckCon);
        self.optimize(Terrain::ShallowWater, 2, 6, 1, CType::CheckCon);
        self.random_and_expand(Terrain::ShallowWater, 3, 3, 20, 5, 3, CType::NoBorn, CType::NoBorn);
        self.out_line(Terrain::ShallowWater, Terrain::Mud, 4, 90, 0, CType::CheckCon);
        
        let mut idx1 = 0;
        while idx1 < self.rand.random_range_int(size_scale, size_scale + 2, RandomType::EmNone, "MakeMap") {
            let w = self.rand.random_range_int(0, size_scale, RandomType::EmNone, "MakeMap");
            let size = self.rand.random_range_int(5 + size_scale, 10 + size_scale, RandomType::EmNone, "MakeMap");
            self.random_line_from_mine_dir(w, size, Terrain::IronOre, CType::NoBorn);
            idx1 += 1;
        }
        
        for t in [Terrain::CopperOre, Terrain::SilverOre] {
            let mut idx = 0;
            while idx < 1 + size_scale {
                let w = self.rand.random_range_int(0, 1, RandomType::EmNone, "MakeMap");
                let size = self.rand.random_range_int(3 + size_scale, 5 + size_scale, RandomType::EmNone, "MakeMap");
                self.random_line_from_mine_dir(w, size, t, CType::NoBorn);
                idx += 1;
            }
        }
        
        let mut idx4 = 0;
        while idx4 < self.rand.random_range_int(1, 3, RandomType::EmNone, "MakeMap") {
            let w = self.rand.random_range_int(0, 1, RandomType::EmNone, "MakeMap");
            let size = self.rand.random_range_int(8, 16, RandomType::EmNone, "MakeMap");
            let stone_t = match self.rand.random_range_int(1, 3, RandomType::EmNone, "MakeMap") { 1 => Terrain::RockGray, _ => Terrain::RockMarble };
            self.random_line_from_mine_dir(w, size, stone_t, CType::NoBorn);
            idx4 += 1;
        }
        
        let mut m = 1;
        while m < self.rand.random_range_int(1, 3, RandomType::EmNone, "MakeMap") {
            let stone_t = match m { 1 => Terrain::RockGray, _ => Terrain::RockMarble };
            let random_count = self.rand.random_range_int(0, 3, RandomType::EmNone, "MakeMap");
            self.random_and_expand(stone_t, random_count, 3, 15 + 3 * size_scale, 0, 3, CType::NoBorn, CType::NoBorn);
            m += 1;
        }
        
        let iron_args = (self.rand.random_range_int(0, size_scale + 1, RandomType::EmNone, "MakeMap"), self.rand.random_range_int(1, size_scale + 1, RandomType::EmNone, "MakeMap"), self.rand.random_range_int(1, 4, RandomType::EmNone, "MakeMap"));
        self.random_and_expand(Terrain::IronOre, iron_args.0, iron_args.1, 13 + size_scale * iron_args.2, 0, 3, CType::NoBorn, CType::NoBorn);
        
        let cop_args = (self.rand.random_range_int(0, size_scale + 1, RandomType::EmNone, "MakeMap"), self.rand.random_range_int(1, size_scale + 1, RandomType::EmNone, "MakeMap"), self.rand.random_range_int(1, 4, RandomType::EmNone, "MakeMap"));
        self.random_and_expand(Terrain::CopperOre, cop_args.0, cop_args.1, 13 + size_scale * cop_args.2, 0, 3, CType::NoBorn, CType::NoBorn);
        
        let sil_args = (self.rand.random_range_int(0, size_scale + 1, RandomType::EmNone, "MakeMap"), self.rand.random_range_int(1, size_scale + 1, RandomType::EmNone, "MakeMap"), self.rand.random_range_int(1, 4, RandomType::EmNone, "MakeMap"));
        self.random_and_expand(Terrain::SilverOre, sil_args.0, sil_args.1, 13 + size_scale * sil_args.2, 0, 3, CType::NoBorn, CType::NoBorn);
        
        let rb_rand_1 = self.rand.random_range_int(2, size_scale + 1, RandomType::EmNone, "MakeMap");
        self.random_and_expand(Terrain::RockBrown, rb_rand_1, size_scale + 1, 13 + size_scale * 4, 0, 3, CType::NoBorn, CType::NoBorn);
        
        for i in 1..3 {
            let t = if i == 1 { Terrain::RockGray } else { Terrain::RockMarble };
            self.out_line(t, Terrain::RockBrown, 1, 50 + 12 * size_scale, 0, CType::NoBorn);
        }
        
        self.out_line(Terrain::IronOre, Terrain::RockBrown, 1, 50 + 12 * size_scale, 0, CType::NoBorn);
        self.out_line(Terrain::CopperOre, Terrain::RockBrown, 1, 50 + 12 * size_scale, 0, CType::NoBorn);
        self.out_line(Terrain::SilverOre, Terrain::RockBrown, 1, 50 + 12 * size_scale, 0, CType::NoBorn);
        
        let out_iron = self.rand.random_range_int(1, 2, RandomType::EmNone, "MakeMap");
        self.out_line(Terrain::IronOre, Terrain::RockBrown, out_iron, 8 + 8 * size_scale, 0, CType::NoBorn);
        let out_cop = self.rand.random_range_int(1, 2, RandomType::EmNone, "MakeMap");
        self.out_line(Terrain::CopperOre, Terrain::RockBrown, out_cop, 8 + 8 * size_scale, 0, CType::NoBorn);
        let out_sil = self.rand.random_range_int(1, 2, RandomType::EmNone, "MakeMap");
        self.out_line(Terrain::SilverOre, Terrain::RockBrown, out_sil, 8 + 8 * size_scale, 0, CType::NoBorn);
        
        for j in 0..3 {
            let t = match j { 1 => Terrain::RockGray, 2 => Terrain::RockMarble, _ => Terrain::RockBrown };
            let out_rand = self.rand.random_range_int(1, size_scale, RandomType::EmNone, "MakeMap");
            self.out_line(t, Terrain::RockBrown, out_rand, 8 + 8 * size_scale, 0, CType::NoBorn);
        }
        
        self.optimize(Terrain::RockBrown, 2, 9, 1, CType::NoBorn);
        
        self.out_line(Terrain::RockBrown, Terrain::StoneLand, 1, 30, 0, CType::CheckCon2);
        self.out_line(Terrain::IronOre, Terrain::StoneLand, 1, 30, 0, CType::CheckCon2);
        self.out_line(Terrain::SilverOre, Terrain::StoneLand, 1, 30, 0, CType::CheckCon2);
        self.out_line(Terrain::CopperOre, Terrain::StoneLand, 1, 30, 0, CType::CheckCon2);
        self.out_line(Terrain::RockBrown, Terrain::StoneLand, 1, 30, 0, CType::CheckCon2);
        self.out_line(Terrain::StoneLand, Terrain::StoneLand, 1, 5, 0, CType::CheckCon);
        
        self.optimize(Terrain::StoneLand, 2, 9, 1, CType::CheckCon);
        self.random_and_expand(Terrain::LingSoil, size_scale, 6, 33, 5, 3, CType::CheckCon, CType::CheckCon);
        
        self.finalize_grid();
    }
}