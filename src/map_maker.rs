use crate::rng::{GMathUtl, RandomType};
use crate::terrain::Terrain;
use std::sync::OnceLock;
static BASE_AROUND_50: OnceLock<Vec<(i32, i32)>> = OnceLock::new();
fn get_base_around_50() -> &'static Vec<(i32, i32)> {
    BASE_AROUND_50.get_or_init(|| {
        let mut list = Vec::with_capacity(10201);
        list.push((0, 0));
        let mut num = 1;
        let mut i = 2;
        let mut dir = 1;
        let mut x = 0;
        let mut y = 0;
        let num5 = 10201;
        
        let mut j = 1;
        while j < num5 {
            let mut current_i = i;
            while current_i > 0 {
                for _ in 0..num {
                    match dir {
                        0 => x += 1,
                        1 => y -= 1,
                        2 => x -= 1,
                        3 => y += 1,
                        _ => {}
                    }
                    list.push((x, y));
                    j += 1;
                    if j >= num5 { return list; }
                }
                dir = (dir + 1) % 4;
                current_i -= 1;
            }
            num += 1;
            i = 2;
        }
        list
    })
}
pub struct MapMaker {
    pub width: i32,
    pub height: i32,
    pub grid: Vec<Terrain>,
    pub rand: GMathUtl,
    pub m_lis_mine_dir: Vec<i32>,
    pub born_space: Vec<bool>,
    pub cache_list: Vec<i32>,
    pub cache_born_line: Vec<bool>,
}
impl MapMaker {
    pub fn new(seed: i32, width: i32, height: i32) -> Self {
        let grid_count = (width * height) as usize;
        Self {
            width, height,
            rand: GMathUtl::new(seed),
            grid: vec![Terrain::Null; grid_count],
            m_lis_mine_dir: Vec::with_capacity(2048),
            born_space: vec![false; grid_count],
            cache_list: Vec::with_capacity(grid_count),
            cache_born_line: vec![false; grid_count],
        }
    }
    pub fn reset(&mut self, seed: i32) {
        self.rand = GMathUtl::new(seed);
        self.m_lis_mine_dir.clear();
    }
    #[inline(always)]
    fn p2key_safe(&self, x: i32, y: i32) -> i32 {
        if x < 0 || x >= self.width || y < 0 || y >= self.height { return -1; }
        y * self.width + x
    }
    #[inline(always)]
    fn key2p(&self, key: i32) -> (i32, i32) { (key % self.width, key / self.width) }
    #[inline(always)]
    fn is_valid_key(&self, key: i32) -> bool { key > 0 && key < self.width * self.height }
    #[inline(always)]
    fn get_grid(&self, key: i32, dir: u8) -> i32 {
        if key == -1 { return -1; }
        let size = self.width;
        let grid_count = size * self.height;
        match dir {
            0 => {
                let num = key + size;
                if num > 0 && num < grid_count { num } else { -1 }
            }
            1 => {
                let num = key - size;
                if num > 0 && num < grid_count { num } else { -1 }
            }
            2 => {
                let num = key - 1;
                if num >= 0 && num < grid_count && (key / size) == (num / size) { num } else { -1 }
            }
            3 => {
                let num = key + 1;
                if num >= 0 && num < grid_count && (key / size) == (num / size) { num } else { -1 }
            }
            4 => {
                let num = self.get_grid(key, 2);
                if num != -1 { self.get_grid(num, 1) } else { -1 }
            }
            5 => {
                let num = self.get_grid(key, 3);
                if num != -1 { self.get_grid(num, 1) } else { -1 }
            }
            6 => {
                let num = self.get_grid(key, 2);
                if num != -1 { self.get_grid(num, 0) } else { -1 }
            }
            7 => {
                let num = self.get_grid(key, 3);
                if num != -1 { self.get_grid(num, 0) } else { -1 }
            }
            _ => -1,
        }
    }
    #[inline(always)]
    fn get_neighbor(&self, key: i32) -> (usize, [i32; 8]) {
        let mut res = [0; 8];
        let mut count = 0;
        
        let dirs = [6, 4, 7, 5, 1, 2, 3, 0];
        
        for &dir in &dirs {
            let n = self.get_grid(key, dir);
            if n != -1 {
                res[count] = n;
                count += 1;
            }
        }
        (count, res)
    }
    fn fill(&mut self, def: Terrain) {
        let grid_count = self.width * self.height;
        for i in 0..grid_count { self.grid[i as usize] = def; }
    }
    fn born_space_random_fill(&mut self, def: Terrain) -> i32 {
        let x = self.rand.random_range_int(self.width / 10 * 4, self.width / 10 * 7, RandomType::EmNone, "MakeMap");
        let y = self.rand.random_range_int(self.height / 10 * 6, self.height / 10 * 7, RandomType::EmNone, "MakeMap");
        let key = self.p2key_safe(x, y);
        if key >= 0 && key < self.width * self.height { self.grid[key as usize] = def; }
        key
    }
    fn make_mine_dir(&mut self, fx: i32, fy: i32) {
        let mut i = 0;
        let mut num = self.rand.random_range_int(0, self.height, RandomType::EmNone, "MakeMineDir");
        let num2 = num;
        let _num3 = self.rand.random_range_int(0, self.height, RandomType::EmNone, "MakeMineDir");
        while i < self.width {
            if self.rand.random_range_int(0, 100, RandomType::EmNone, "MakeMineDir") < 10 { i -= 1; } else { i += 1; }
            if num2 > self.height / 2 { num += self.rand.random_range_int(-fy, fx, RandomType::EmNone, "MakeMineDir"); }
            else { num += self.rand.random_range_int(-fx, fy, RandomType::EmNone, "MakeMineDir"); }
            let key = self.p2key_safe(i, num);
            if self.is_valid_key(key) { 
                self.m_lis_mine_dir.push(key); 
            }
        }
    }
    fn random_line_from_mine_dir<F>(&mut self, w: i32, size: i32, def: Terrain, mut con: F)
    where F: FnMut(i32, Terrain) -> bool {
        if self.m_lis_mine_dir.is_empty() { return; }
        let mut num = self.rand.random_range_int(0, self.m_lis_mine_dir.len() as i32, RandomType::EmNone, "MakeMap") as usize;
        for _ in 0..size {
            if num >= self.m_lis_mine_dir.len() {
                num = self.rand.random_range_int(0, self.m_lis_mine_dir.len() as i32, RandomType::EmNone, "MakeMap") as usize;
            }
            let mut key = self.m_lis_mine_dir[num];
            num += 1;
            let (n_count, n_keys) = self.get_neighbor(key);
            if n_count > 0 {
                key = n_keys[self.rand.random_range_int(0, n_count as i32, RandomType::EmNone, "MakeMap") as usize];
            }
            if key >= 0 && key < self.width * self.height {
                let t = self.grid[key as usize];
                if con(key, t) { self.grid[key as usize] = def; }
            }
        }
        self.out_line(def, def, w, 4, 0, con);
    }
    fn out_line<F>(&mut self, def: Terrain, line: Terrain, w: i32, lv: i32, maxcount: i32, mut con: F)
    where F: FnMut(i32, Terrain) -> bool {
        let mut list = std::mem::take(&mut self.cache_list);
        list.clear();
        let grid_count = self.width * self.height;
        for i in (1..grid_count).rev() {
            if self.grid[i as usize] == def { list.push(i); }
        }
        let limit = (w * 2 + 1) * (w * 2 + 1);
        let base_around_50 = get_base_around_50();
        let mut current_maxcount = maxcount;
        for &key in &list {
            let cx = key % self.width;
            let cy = key / self.width;
            
            for i in 0..limit {
                if i as usize >= base_around_50.len() { break; }
                let (dx, dy) = base_around_50[i as usize];
                let nx = cx + dx;
                let ny = cy + dy;
                
                let node = self.p2key_safe(nx, ny);
                if node != -1 {
                    if self.rand.random_range_int(0, 100, RandomType::EmNone, "OutLine") > lv { continue; }
                    let t = self.grid[node as usize];
                    if con(node, t) {
                        self.grid[node as usize] = line;
                        current_maxcount -= 1;
                        if current_maxcount == 0 { break; } 
                    }
                }
            }
        }
        
        self.cache_list = list;
    }
    fn get_count<F>(&self, key: i32, r: i32, def: Terrain, mut con: F) -> i32 
    where F: FnMut(i32, Terrain) -> bool {
        let mut count = 0;
        if r == 1 {
            let (n_count, n_keys) = self.get_neighbor(key);
            for i in 0..n_count {
                let k = n_keys[i];
                let t = self.grid[k as usize];
                if t == def && con(k, t) { count += 1; }
            }
        }
        count
    }
    fn optimize<F>(&mut self, def: Terrain, opt_min: i32, opt_max: i32, maxcount: i32, mut con: F)
    where F: FnMut(i32, Terrain) -> bool {
        let grid_count = self.width * self.height;
        let mut list = std::mem::take(&mut self.cache_list);
        
        for _ in 0..maxcount {
            list.clear();
            for j in 0..grid_count {
                let t = self.grid[j as usize];
                let count = self.get_count(j, 1, def, |_, _| true);
                if count >= opt_min && count <= opt_max && con(j, t) { list.push(j); }
            }
            for &k in &list { self.grid[k as usize] = def; }
        }
        
        self.cache_list = list;
    }
    fn random_and_expand<F, E>(&mut self, def: Terrain, randomcount: i32, expandcount: i32, expand_lv: i32, opt_lv: i32, opt_count: i32, mut con: F, mut econ: E)
    where F: FnMut(i32, Terrain) -> bool, E: FnMut(i32, Terrain) -> bool {
        let rcount = std::cmp::max(1, randomcount);
        for _ in 0..rcount {
            let x = self.rand.random_range_int(0, self.width, RandomType::EmNone, "MakeMap");
            let y = self.rand.random_range_int(0, self.height, RandomType::EmNone, "MakeMap");
            let key = self.p2key_safe(x, y);
            if key >= 0 && key < self.width * self.height {
                let t = self.grid[key as usize];
                if con(key, t) { self.grid[key as usize] = def; }
            }
        }
        let grid_count = self.width * self.height;
        let mut flag = true;
        for _ in 0..expandcount {
            if flag {
                for j in 0..grid_count {
                    if self.grid[j as usize] == def && self.rand.random_range_int(0, 100, RandomType::EmNone, "MakeMap") <= expand_lv {
                        let (n_count, n_keys) = self.get_neighbor(j);
                        for i in 0..n_count {
                            let nk = n_keys[i];
                            let t = self.grid[nk as usize];
                            if econ(nk, t) { self.grid[nk as usize] = def; }
                        }
                    }
                }
                flag = false;
            } else {
                for j in (0..grid_count).rev() {
                    if self.grid[j as usize] == def && self.rand.random_range_int(0, 100, RandomType::EmNone, "MakeMap") <= expand_lv {
                        let (n_count, n_keys) = self.get_neighbor(j);
                        for i in 0..n_count {
                            let nk = n_keys[i];
                            let t = self.grid[nk as usize];
                            if econ(nk, t) { self.grid[nk as usize] = def; }
                        }
                    }
                }
                flag = true;
            }
        }
        if opt_lv > 0 { self.optimize(def, opt_lv, opt_count, 1, econ); }
    }
    pub fn make_map(&mut self) {
        let size_scale = std::cmp::max(1, self.width / 64);
        let grid_count = self.width * self.height;
        self.make_mine_dir(2, 3);
        self.make_mine_dir(2, 3);
        self.make_mine_dir(2, 3);
        self.fill(Terrain::Soil);
        let _borncenter = self.born_space_random_fill(Terrain::FertileSoil);
        self.out_line(Terrain::FertileSoil, Terrain::FertileSoil, 2, 100, 0, |_, _| true);
        self.out_line(Terrain::FertileSoil, Terrain::FertileSoil, 5, 20, 0, |_, _| true);
        self.optimize(Terrain::FertileSoil, 2, 4, 1, |_, _| true);
        let mut map_born_space = std::mem::take(&mut self.cache_list);
        map_born_space.clear();
        for i in 1..grid_count {
            if self.grid[i as usize] == Terrain::FertileSoil { map_born_space.push(i); }
        }
        self.born_space.fill(false);
        for &k in &map_born_space {
            self.born_space[k as usize] = true;
        }
        self.cache_list = map_born_space;
        self.out_line(Terrain::FertileSoil, Terrain::FertileSoil, 2, 100, 0, |_, _| true);
        let mut m_born_space = std::mem::take(&mut self.cache_list);
        m_born_space.clear();
        for i in 1..grid_count {
            if self.grid[i as usize] == Terrain::FertileSoil {
                m_born_space.push(i);
                self.grid[i as usize] = Terrain::Soil;
            }
        }
        for &num in &m_born_space {
            if !self.born_space[num as usize] {
                self.grid[num as usize] = Terrain::Soil;
            }
        }
        self.cache_list = m_born_space;
        let mut born_line = std::mem::take(&mut self.cache_born_line);
        born_line.fill(false);
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
                if k >= 0 { born_line[k as usize] = true; }
                j += 1;
            }
        }
        self.random_and_expand(Terrain::FertileSoil, 20, 4, 30, 5, 3, |_, _| true, |_, _| true);
        let bs = std::mem::take(&mut self.born_space);
        
        {
            let bs_ref = &bs;
            let bl_ref = &born_line;
            let check_con_no_born = |k: i32, t: Terrain| -> bool {
                (t == Terrain::Soil || t == Terrain::FertileSoil) && !bs_ref[k as usize] && !bl_ref[k as usize]
            };
            let check_con = |_: i32, t: Terrain| -> bool {
                t == Terrain::Soil || t == Terrain::FertileSoil || t == Terrain::LingSoil
            };
            self.random_and_expand(Terrain::DDepthWater, size_scale - 1, 2 * size_scale - 1, 13 + 6 * size_scale, 5, 3, check_con_no_born, check_con_no_born);
            self.out_line(Terrain::DDepthWater, Terrain::DepthWater, 1, 100, 0, check_con);
            self.out_line(Terrain::DepthWater, Terrain::DepthWater, 1, 10 + 6 * size_scale, 0, check_con_no_born);
            self.out_line(Terrain::DepthWater, Terrain::ShallowWater, 4, 50 + 12 * size_scale, 0, check_con);
            self.optimize(Terrain::ShallowWater, 2, 6, 1, check_con);
            self.random_and_expand(Terrain::ShallowWater, 3, 3, 20, 5, 3, check_con_no_born, check_con_no_born);
            self.out_line(Terrain::ShallowWater, Terrain::Mud, 4, 90, 0, check_con);
            let mut idx1 = 0;
            while idx1 < self.rand.random_range_int(size_scale, size_scale + 2, RandomType::EmNone, "MakeMap") {
                let w = self.rand.random_range_int(0, size_scale, RandomType::EmNone, "MakeMap");
                let size = self.rand.random_range_int(5 + size_scale, 10 + size_scale, RandomType::EmNone, "MakeMap");
                self.random_line_from_mine_dir(w, size, Terrain::IronOre, check_con_no_born);
                idx1 += 1;
            }
            let mut idx2 = 0;
            let limit2 = 1 + size_scale;
            while idx2 < limit2 {
                let w = self.rand.random_range_int(0, 1, RandomType::EmNone, "MakeMap");
                let size = self.rand.random_range_int(3 + size_scale, 5 + size_scale, RandomType::EmNone, "MakeMap");
                self.random_line_from_mine_dir(w, size, Terrain::CopperOre, check_con_no_born);
                idx2 += 1;
            }
            let mut idx3 = 0;
            let limit3 = 1 + size_scale;
            while idx3 < limit3 {
                let w = self.rand.random_range_int(0, 1, RandomType::EmNone, "MakeMap");
                let size = self.rand.random_range_int(3 + size_scale, 5 + size_scale, RandomType::EmNone, "MakeMap");
                self.random_line_from_mine_dir(w, size, Terrain::SilverOre, check_con_no_born);
                idx3 += 1;
            }
            let mut idx4 = 0;
            while idx4 < self.rand.random_range_int(1, 3, RandomType::EmNone, "MakeMap") {
                let w = self.rand.random_range_int(0, 1, RandomType::EmNone, "MakeMap");
                let size = self.rand.random_range_int(8, 16, RandomType::EmNone, "MakeMap");
                let stone_t = match self.rand.random_range_int(1, 3, RandomType::EmNone, "MakeMap") {
                    1 => Terrain::RockGray,
                    _ => Terrain::RockMarble,
                };
                self.random_line_from_mine_dir(w, size, stone_t, check_con_no_born);
                idx4 += 1;
            }
            let mut m = 1;
            while m < self.rand.random_range_int(1, 3, RandomType::EmNone, "MakeMap") {
                let stone_t = match m {
                    1 => Terrain::RockGray,
                    _ => Terrain::RockMarble,
                };
                let random_count = self.rand.random_range_int(0, 3, RandomType::EmNone, "MakeMap");
                self.random_and_expand(stone_t, random_count, 3, 15 + 3 * size_scale, 0, 3, check_con_no_born, check_con_no_born);
                m += 1;
            }
            let iron_rand_1 = self.rand.random_range_int(0, size_scale + 1, RandomType::EmNone, "MakeMap");
            let iron_rand_2 = self.rand.random_range_int(1, size_scale + 1, RandomType::EmNone, "MakeMap");
            let iron_rand_3 = self.rand.random_range_int(1, 4, RandomType::EmNone, "MakeMap");
            self.random_and_expand(Terrain::IronOre, iron_rand_1, iron_rand_2, 13 + size_scale * iron_rand_3, 0, 3, check_con_no_born, check_con_no_born);
            let cop_rand_1 = self.rand.random_range_int(0, size_scale + 1, RandomType::EmNone, "MakeMap");
            let cop_rand_2 = self.rand.random_range_int(1, size_scale + 1, RandomType::EmNone, "MakeMap");
            let cop_rand_3 = self.rand.random_range_int(1, 4, RandomType::EmNone, "MakeMap");
            self.random_and_expand(Terrain::CopperOre, cop_rand_1, cop_rand_2, 13 + size_scale * cop_rand_3, 0, 3, check_con_no_born, check_con_no_born);
            let sil_rand_1 = self.rand.random_range_int(0, size_scale + 1, RandomType::EmNone, "MakeMap");
            let sil_rand_2 = self.rand.random_range_int(1, size_scale + 1, RandomType::EmNone, "MakeMap");
            let sil_rand_3 = self.rand.random_range_int(1, 4, RandomType::EmNone, "MakeMap");
            self.random_and_expand(Terrain::SilverOre, sil_rand_1, sil_rand_2, 13 + size_scale * sil_rand_3, 0, 3, check_con_no_born, check_con_no_born);
            let rbn_rand_1 = self.rand.random_range_int(2, size_scale + 1, RandomType::EmNone, "MakeMap");
            self.random_and_expand(Terrain::RockBrown, rbn_rand_1, size_scale + 1, 13 + size_scale * 4, 0, 3, check_con_no_born, check_con_no_born);
            for i in 1..3 {
                let t = if i == 1 { Terrain::RockGray } else { Terrain::RockMarble };
                self.out_line(t, Terrain::RockBrown, 1, 50 + 12 * size_scale, 0, check_con_no_born);
            }
            self.out_line(Terrain::IronOre, Terrain::RockBrown, 1, 50 + 12 * size_scale, 0, check_con_no_born);
            self.out_line(Terrain::CopperOre, Terrain::RockBrown, 1, 50 + 12 * size_scale, 0, check_con_no_born);
            self.out_line(Terrain::SilverOre, Terrain::RockBrown, 1, 50 + 12 * size_scale, 0, check_con_no_born);
            let out_iron = self.rand.random_range_int(1, 2, RandomType::EmNone, "MakeMap");
            self.out_line(Terrain::IronOre, Terrain::RockBrown, out_iron, 8 + 8 * size_scale, 0, check_con_no_born);
            let out_cop = self.rand.random_range_int(1, 2, RandomType::EmNone, "MakeMap");
            self.out_line(Terrain::CopperOre, Terrain::RockBrown, out_cop, 8 + 8 * size_scale, 0, check_con_no_born);
            let out_sil = self.rand.random_range_int(1, 2, RandomType::EmNone, "MakeMap");
            self.out_line(Terrain::SilverOre, Terrain::RockBrown, out_sil, 8 + 8 * size_scale, 0, check_con_no_born);
            for j in 0..3 {
                let t = match j {
                    1 => Terrain::RockGray,
                    2 => Terrain::RockMarble,
                    _ => Terrain::RockBrown,
                };
                let out_rand = self.rand.random_range_int(1, size_scale, RandomType::EmNone, "MakeMap");
                self.out_line(t, Terrain::RockBrown, out_rand, 8 + 8 * size_scale, 0, check_con_no_born);
            }
            self.optimize(Terrain::RockBrown, 2, 9, 1, check_con_no_born);
            let check_con_2 = |k: i32, t: Terrain| -> bool {
                t != Terrain::IronOre && t != Terrain::CopperOre && t != Terrain::SilverOre &&
                t != Terrain::RockBrown && t != Terrain::RockGray && (t != Terrain::RockMarble || bs_ref[k as usize] || bl_ref[k as usize])
            };
            self.out_line(Terrain::RockBrown, Terrain::StoneLand, 1, 30, 0, check_con_2);
            self.out_line(Terrain::IronOre, Terrain::StoneLand, 1, 30, 0, check_con_2);
            self.out_line(Terrain::SilverOre, Terrain::StoneLand, 1, 30, 0, check_con_2);
            self.out_line(Terrain::CopperOre, Terrain::StoneLand, 1, 30, 0, check_con_2);
            self.out_line(Terrain::RockBrown, Terrain::StoneLand, 1, 30, 0, check_con_2);
            self.out_line(Terrain::StoneLand, Terrain::StoneLand, 1, 5, 0, check_con);
            self.optimize(Terrain::StoneLand, 2, 9, 1, check_con);
            self.random_and_expand(Terrain::LingSoil, size_scale, 6, 33, 5, 3, check_con, check_con);
        }
        self.born_space = bs;
        self.cache_born_line = born_line;
    }
}