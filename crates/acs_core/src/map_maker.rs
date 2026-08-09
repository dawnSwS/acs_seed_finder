use crate::{
    constants::{get_base_around_50, CType},
    rng::{GMathUtl, RandomType::EmNone},
    terrain::Terrain::{self, *},
};

pub struct MapMaker {
    pub w: i32,
    pub h: i32,
    pub grid: Vec<Terrain>,
    pub rand: GMathUtl,
    pub dirs: Vec<i32>,
    pub b_space: Vec<bool>,
    pub c_list: Vec<i32>,
    pub b_line: Vec<bool>,
    pub bb: [[u32; 1152]; 20],
}

impl MapMaker {
    pub fn new(seed: i32, w: i32, h: i32) -> Self {
        let count = (w * h) as usize;
        Self {
            w,
            h,
            rand: GMathUtl::new(seed),
            grid: vec![Null; count],
            dirs: Vec::with_capacity(2048),
            b_space: vec![false; count],
            c_list: vec![],
            b_line: vec![false; count],
            bb: [[0; 1152]; 20],
        }
    }
    pub fn reset(&mut self, s: i32) {
        self.rand = GMathUtl::new(s);
        self.dirs.clear();
        self.bb.iter_mut().for_each(|b| b.fill(0));
        self.grid.fill(Null);
        self.b_space.fill(false);
        self.b_line.fill(false);
    }
    #[inline]
    pub fn p2key_safe(&self, x: i32, y: i32) -> i32 {
        if x < 0 || x >= self.w || y < 0 || y >= self.h {
            -1
        } else {
            y * self.w + x
        }
    }
    #[inline]
    pub fn is_valid_key(&self, key: i32) -> bool {
        key > 0 && key < self.w * self.h
    }
    #[inline]
    pub fn set_mask_raw(&mut self, t: usize, w: usize, m: u32) {
        for i in 1..=15 {
            if i == t {
                self.bb[i][w] |= m;
            } else {
                self.bb[i][w] &= !m;
            }
        }
    }
    #[inline]
    pub fn set_mask(&mut self, t: Terrain, w: usize, m: u32) {
        self.set_mask_raw(t as usize, w, m);
    }
    #[inline]
    pub fn get_ctype_mask(&self, w: usize, c: CType) -> u32 {
        let (s, rem) = (((self.w + 31) / 32) as usize, self.w as u32 % 32);
        let mut base = match c {
            CType::AllTrue => !0,
            CType::NoBorn => {
                (self.bb[Soil as usize][w] | self.bb[FertileSoil as usize][w])
                    & !(self.bb[BornSpace as usize][w] | self.bb[BornLine as usize][w])
            }
            CType::CheckCon => {
                self.bb[Soil as usize][w]
                    | self.bb[FertileSoil as usize][w]
                    | self.bb[LingSoil as usize][w]
            }
            CType::CheckCon2 => {
                !(self.bb[IronOre as usize][w]
                    | self.bb[CopperOre as usize][w]
                    | self.bb[SilverOre as usize][w]
                    | self.bb[RockBrown as usize][w]
                    | self.bb[RockGray as usize][w]
                    | (self.bb[RockMarble as usize][w]
                        & !(self.bb[BornSpace as usize][w] | self.bb[BornLine as usize][w])))
            }
        };
        if rem != 0 && w % s == s - 1 {
            base &= (1 << rem) - 1;
        }
        base
    }
    #[inline]
    pub fn get_grid(&self, k: i32, d: u8) -> i32 {
        if k < 0 {
            return -1;
        }
        let (w, c) = (self.w, self.w * self.h);
        match d {
            0 => {
                let n = k + w;
                if n > 0 && n < c {
                    n
                } else {
                    -1
                }
            }
            1 => {
                let n = k - w;
                if n > 0 && n < c {
                    n
                } else {
                    -1
                }
            }
            2 => {
                let n = k - 1;
                if n >= 0 && n < c && k / w == n / w {
                    n
                } else {
                    -1
                }
            }
            3 => {
                let n = k + 1;
                if n >= 0 && n < c && k / w == n / w {
                    n
                } else {
                    -1
                }
            }
            4 => {
                let n = self.get_grid(k, 2);
                if n != -1 {
                    self.get_grid(n, 1)
                } else {
                    -1
                }
            }
            5 => {
                let n = self.get_grid(k, 3);
                if n != -1 {
                    self.get_grid(n, 1)
                } else {
                    -1
                }
            }
            6 => {
                let n = self.get_grid(k, 2);
                if n != -1 {
                    self.get_grid(n, 0)
                } else {
                    -1
                }
            }
            7 => {
                let n = self.get_grid(k, 3);
                if n != -1 {
                    self.get_grid(n, 0)
                } else {
                    -1
                }
            }
            _ => -1,
        }
    }
    #[inline]
    fn get_cpu_neighbor(&self, k: i32, out: &mut [i32; 8]) -> usize {
        [6, 4, 7, 5, 1, 2, 3, 0]
            .into_iter()
            .filter_map(|d| {
                let n = self.get_grid(k, d);
                (n != -1).then_some(n)
            })
            .fold(0, |c, n| {
                out[c] = n;
                c + 1
            })
    }
    fn fill(&mut self, def: Terrain) {
        let (s, rem) = (((self.w + 31) / 32) as usize, self.w as u32 % 32);
        for t in 1..=15 {
            if t == def as usize {
                for w in 0..self.bb[t].len() {
                    self.bb[t][w] = if rem != 0 && w % s == s - 1 {
                        (1 << rem) - 1
                    } else {
                        !0
                    };
                }
            } else {
                self.bb[t].fill(0);
            }
        }
    }
    fn make_mine_dir(&mut self, fx: i32, fy: i32) {
        let (mut i, mut num) = (0, self.rand.random_range_int(0, self.h, EmNone, ""));
        let num2 = num;
        let _ = self.rand.random_range_int(0, self.h, EmNone, "");
        while i < self.w {
            i += if self.rand.random_range_int(0, 100, EmNone, "") < 10 {
                -1
            } else {
                1
            };
            num += self.rand.random_range_int(
                if num2 > self.h / 2 { -fy } else { -fx },
                if num2 > self.h / 2 { fx } else { fy },
                EmNone,
                "",
            );
            let k = self.p2key_safe(i, num);
            if self.is_valid_key(k) {
                self.dirs.push(k);
            }
        }
    }
    fn random_line_from_mine_dir(&mut self, w: i32, size: i32, def: Terrain, ctype: CType) {
        if self.dirs.is_empty() {
            return;
        }
        let (s, mut num) = (
            ((self.w + 31) / 32) as usize,
            self.rand
                .random_range_int(0, self.dirs.len() as i32, EmNone, "") as usize,
        );
        for _ in 0..size {
            if num >= self.dirs.len() {
                num = self
                    .rand
                    .random_range_int(0, self.dirs.len() as i32, EmNone, "")
                    as usize;
            }
            let mut k = self.dirs[num];
            num += 1;
            let mut vk = [0; 8];
            let n = self.get_cpu_neighbor(k, &mut vk);
            if n > 0 {
                k = vk[self.rand.random_range_int(0, n as i32, EmNone, "") as usize];
            }
            if k >= 0 && k < self.w * self.h {
                let (nw, bit) = (
                    (k / self.w) as usize * s + (k % self.w) as usize / 32,
                    1 << (k % self.w % 32),
                );
                if (self.get_ctype_mask(nw, ctype) & bit) != 0 {
                    self.set_mask(def, nw, bit);
                }
            }
        }
        self.out_line(def, def, w, 4, 0, ctype);
    }
    pub fn out_line(
        &mut self,
        src: Terrain,
        tgt: Terrain,
        w: i32,
        lv: i32,
        mut maxc: i32,
        ctype: CType,
    ) {
        {
            let (l, u) = self.bb.split_at_mut(Tmp1 as usize);
            u[0].copy_from_slice(&l[src as usize]);
        }
        let (lim, base, s) = (
            ((w * 2 + 1).pow(2)) as usize,
            get_base_around_50(),
            ((self.w + 31) / 32) as usize,
        );
        for word in (0..s * self.h as usize).rev() {
            let mut m = self.bb[Tmp1 as usize][word] & if word == 0 { !1 } else { !0 };
            while m != 0 {
                let b = 31 - m.leading_zeros();
                m ^= 1 << b;
                let (cx, cy) = (((word % s) * 32 + b as usize) as i32, (word / s) as i32);
                if cx >= self.w {
                    continue;
                }
                for &(dx, dy) in base.iter().take(lim) {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if nx >= 0 && nx < self.w && ny >= 0 && ny < self.h {
                        if self.rand.random_range_int(0, 100, EmNone, "") <= lv {
                            let (nw, nbit) = (ny as usize * s + nx as usize / 32, 1 << (nx % 32));
                            if (self.get_ctype_mask(nw, ctype) & nbit) != 0 {
                                self.set_mask(tgt, nw, nbit);
                                maxc -= 1;
                                if maxc == 0 {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    pub fn optimize(&mut self, def: Terrain, opt_min: i32, opt_max: i32, maxc: i32, ctype: CType) {
        let (d, s, h) = (def as usize, ((self.w + 31) / 32) as usize, self.h as usize);
        for _ in 0..maxc {
            for w in 0..s * h {
                self.bb[Tmp2 as usize][w] = self.get_ctype_mask(w, ctype);
            }
            {
                let (l, r) = self.bb.split_at_mut(Tmp1 as usize);
                let (rm, rr) = r.split_first_mut().unwrap();
                let r19 = &rr[0];
                let layer = &l[d];
                for y in 0..h {
                    let (ro, up, dn) = (y * s, y.saturating_sub(1) * s, (y + 1).min(h - 1) * s);
                    for xw in 0..s {
                        let w = ro + xw;
                        let m = r19[w];
                        if m == 0 {
                            rm[w] = 0;
                            continue;
                        }
                        macro_rules! f {
                            ($o:expr, $up:expr) => {{
                                let mut c = layer[$o + xw];
                                if $up && y == 1 && xw == 0 {
                                    c &= !1;
                                }
                                (
                                    if xw > 0 {
                                        (c << 1) | (layer[$o + xw - 1] >> 31)
                                    } else {
                                        c << 1
                                    },
                                    c,
                                    if xw < s - 1 {
                                        (c >> 1) | (layer[$o + xw + 1] << 31)
                                    } else {
                                        c >> 1
                                    },
                                )
                            }};
                        }
                        let (ml, _, mr) = f!(ro, false);
                        let (ul, uc, ur) = if y > 0 { f!(up, true) } else { (0, 0, 0) };
                        let (dl, dc, dr) = if y < h - 1 { f!(dn, false) } else { (0, 0, 0) };
                        macro_rules! fa {
                            ($a:expr, $b:expr, $c:expr) => {{
                                let t = $a ^ $b;
                                (t ^ $c, ($a & $b) | (t & $c))
                            }};
                        }
                        let (s0, c1) = (ml ^ mr, ml & mr);
                        let (s1, c2) = fa!(ul, uc, ur);
                        let (s2, c3) = fa!(dl, dc, dr);
                        let (s01, c11) = fa!(s0, s1, s2);
                        let (s10, c20) = fa!(c1, c2, c3);
                        let (s11, c21) = (s10 ^ c11, s10 & c11);
                        let (s20, c30) = (c20 ^ c21, c20 & c21);
                        let (b0, b1, b2, b3) = (s01, s11, s20, c30);
                        let mask = if opt_min == 2 && opt_max == 4 {
                            (!b3 & !b2 & b1) | (!b3 & b2 & !b1 & !b0)
                        } else if opt_min == 2 && opt_max == 6 {
                            (!b3 & !b2 & b1) | (!b3 & b2 & !b1) | (!b3 & b2 & b1 & !b0)
                        } else if opt_min == 2 && (opt_max == 8 || opt_max == 9) {
                            b3 | b2 | b1
                        } else {
                            let mut m = 0;
                            for i in 0..32 {
                                let c = ((b3 >> i) & 1) * 8
                                    + ((b2 >> i) & 1) * 4
                                    + ((b1 >> i) & 1) * 2
                                    + ((b0 >> i) & 1);
                                if c >= opt_min as u32 && c <= opt_max as u32 {
                                    m |= 1 << i;
                                }
                            }
                            m
                        };
                        rm[w] = mask & m;
                    }
                }
            }
            for w in 0..s * h {
                let diff = self.bb[Tmp1 as usize][w] & !self.bb[d][w];
                if diff != 0 {
                    self.set_mask(def, w, diff);
                }
            }
        }
    }
    pub fn random_and_expand(
        &mut self,
        def: Terrain,
        rc: i32,
        ec: i32,
        elv: i32,
        olv: i32,
        oc: i32,
        ctype: CType,
        ectype: CType,
    ) {
        let (d, s, words) = (
            def as usize,
            ((self.w + 31) / 32) as usize,
            ((self.w + 31) / 32 * self.h) as usize,
        );
        for _ in 0..rc.max(1) {
            let (x, y) = (
                self.rand.random_range_int(0, self.w, EmNone, ""),
                self.rand.random_range_int(0, self.h, EmNone, ""),
            );
            let nw = y as usize * s + x as usize / 32;
            let bit = 1 << (x % 32);
            if (self.get_ctype_mask(nw, ctype) & bit) != 0 {
                self.set_mask(def, nw, bit);
            }
        }
        let mut flag = true;
        for _ in 0..ec {
            macro_rules! step {
                ($w_iter:expr, $b_iter:expr) => {
                    for w in $w_iter {
                        for b in $b_iter {
                            if (self.bb[d][w] & (1 << b)) != 0 && w % s * 32 + b < self.w as usize {
                                let k = (w / s * self.w as usize + w % s * 32 + b) as i32;
                                if self.rand.random_range_int(0, 100, EmNone, "") <= elv {
                                    let mut nk = [0; 8];
                                    for i in 0..self.get_cpu_neighbor(k, &mut nk) {
                                        let (nw, nbit) = (
                                            (nk[i] / self.w) as usize * s
                                                + (nk[i] % self.w) as usize / 32,
                                            1 << (nk[i] % self.w % 32),
                                        );
                                        if (self.get_ctype_mask(nw, ectype) & nbit) != 0 {
                                            self.set_mask(def, nw, nbit);
                                        }
                                    }
                                }
                            }
                        }
                    }
                };
            }
            if flag {
                step!(0..words, 0..32);
            } else {
                step!((0..words).rev(), (0..32).rev());
            }
            flag = !flag;
        }
        if olv > 0 {
            self.optimize(def, olv, oc, 1, ectype);
        }
    }
    fn finalize_grid(&mut self) {
        let (s, w) = (((self.w + 31) / 32) as usize, self.w as usize);
        self.grid.fill(Null);
        self.b_space.fill(false);
        self.b_line.fill(false);
        for y in 0..self.h as usize {
            for x in 0..w {
                let (k, nw, bit) = (y * w + x, y * s + x / 32, 1 << (x % 32));
                self.grid[k] = [
                    LingSoil,
                    StoneLand,
                    RockBrown,
                    RockGray,
                    RockMarble,
                    IronOre,
                    CopperOre,
                    SilverOre,
                    Mud,
                    ShallowWater,
                    DepthWater,
                    DDepthWater,
                    FertileSoil,
                    Soil,
                ]
                .into_iter()
                .find(|&t| (self.bb[t as usize][nw] & bit) != 0)
                .unwrap_or(Null);
            }
        }
        for nw in 0..s * self.h as usize {
            let mut m1 = self.bb[16][nw];
            while m1 != 0 {
                let b = m1.trailing_zeros();
                m1 ^= 1 << b;
                let cx = nw % s * 32 + b as usize;
                if cx < w {
                    self.b_space[(nw / s) * w + cx] = true;
                }
            }
            let mut m2 = self.bb[17][nw];
            while m2 != 0 {
                let b = m2.trailing_zeros();
                m2 ^= 1 << b;
                let cx = nw % s * 32 + b as usize;
                if cx < w {
                    self.b_line[(nw / s) * w + cx] = true;
                }
            }
        }
    }

    // ====== 解耦重组：按生命周期流转切割的方法阶段 ======
    pub fn step1_build_base_topology(&mut self) {
        let (s, _) = (((self.w + 31) / 32) as usize, self.w as usize);
        for _ in 0..3 {
            self.make_mine_dir(2, 3);
        }
        self.fill(Soil);
        let (kx, ky) = (
            self.rand
                .random_range_int(self.w / 10 * 4, self.w / 10 * 7, EmNone, ""),
            self.rand
                .random_range_int(self.h / 10 * 6, self.h / 10 * 7, EmNone, ""),
        );
        if self.p2key_safe(kx, ky) >= 0 {
            self.set_mask(
                FertileSoil,
                ky as usize * s + kx as usize / 32,
                1 << (kx % 32),
            );
        }
        self.out_line(FertileSoil, FertileSoil, 2, 100, 0, CType::AllTrue);
        self.out_line(FertileSoil, FertileSoil, 5, 20, 0, CType::AllTrue);
    }
    pub fn step2_post_gpu_expansion(&mut self, skip_opt: bool) {
        let (sc, s) = (1.max(self.w / 64), ((self.w + 31) / 32) as usize);
        for i in 0..s * self.h as usize {
            let mut m = self.bb[FertileSoil as usize][i];
            if i == 0 {
                m &= !1;
            }
            self.bb[16][i] = m;
        }
        self.out_line(FertileSoil, FertileSoil, 2, 100, 0, CType::AllTrue);
        for i in 0..s * self.h as usize {
            let mut m = self.bb[FertileSoil as usize][i];
            if i == 0 {
                m &= !1;
            }
            if m != 0 {
                self.set_mask_raw(Soil as usize, i, m);
            }
        }
        self.bb[17].fill(0);
        for i in 0..4 {
            let n = self.rand.random_range_int(
                (self.w as f32 * 0.3) as i32,
                (self.w as f32 * 0.6) as i32,
                EmNone,
                "",
            );
            let mut j = 0;
            while j < self.rand.random_range_int(5, 15, EmNone, "") {
                let k = match i {
                    0 => self.p2key_safe(0, n + j),
                    1 => self.p2key_safe(self.w - 1, n + j),
                    2 => self.p2key_safe(n + j, 0),
                    _ => self.p2key_safe(n + j, self.w - 1),
                };
                if k >= 0 {
                    self.bb[17][(k as usize / self.w as usize) * s
                        + (k as usize % self.w as usize) / 32] |=
                        1u32 << (((k as usize % self.w as usize) as u32) % 32);
                }
                j += 1;
            }
        }
        self.random_and_expand(
            FertileSoil,
            20,
            4,
            30,
            if skip_opt { 0 } else { 5 },
            3,
            CType::AllTrue,
            CType::AllTrue,
        );
        self.random_and_expand(
            DDepthWater,
            sc - 1,
            2 * sc - 1,
            13 + 6 * sc,
            if skip_opt { 0 } else { 5 },
            3,
            CType::NoBorn,
            CType::NoBorn,
        );
        self.out_line(DDepthWater, DepthWater, 1, 100, 0, CType::CheckCon);
        self.out_line(DepthWater, DepthWater, 1, 10 + 6 * sc, 0, CType::NoBorn);
        self.out_line(
            DepthWater,
            ShallowWater,
            4,
            50 + 12 * sc,
            0,
            CType::CheckCon,
        );
    }
    pub fn step3_post_gpu_expansion(&mut self, skip_opt: bool) {
        let sc = 1.max(self.w / 64);
        self.random_and_expand(
            ShallowWater,
            3,
            3,
            20,
            if skip_opt { 0 } else { 5 },
            3,
            CType::NoBorn,
            CType::NoBorn,
        );
        self.out_line(ShallowWater, Mud, 4, 90, 0, CType::CheckCon);
        let mut idx1 = 0;
        while idx1 < self.rand.random_range_int(sc, sc + 2, EmNone, "") {
            let (sz, x) = (
                self.rand.random_range_int(0, sc, EmNone, ""),
                self.rand.random_range_int(5 + sc, 10 + sc, EmNone, ""),
            );
            self.random_line_from_mine_dir(sz, x, IronOre, CType::NoBorn);
            idx1 += 1;
        }
        for t in [CopperOre, SilverOre] {
            for _ in 0..1 + sc {
                let (wr, x) = (
                    self.rand.random_range_int(0, 1, EmNone, ""),
                    self.rand.random_range_int(3 + sc, 5 + sc, EmNone, ""),
                );
                self.random_line_from_mine_dir(wr, x, t, CType::NoBorn);
            }
        }
        let mut idx4 = 0;
        while idx4 < self.rand.random_range_int(1, 3, EmNone, "") {
            let (wr, x, st) = (
                self.rand.random_range_int(0, 1, EmNone, ""),
                self.rand.random_range_int(8, 16, EmNone, ""),
                if self.rand.random_range_int(1, 3, EmNone, "") == 1 {
                    RockGray
                } else {
                    RockMarble
                },
            );
            self.random_line_from_mine_dir(wr, x, st, CType::NoBorn);
            idx4 += 1;
        }
        let mut m = 1;
        while m < self.rand.random_range_int(1, 3, EmNone, "") {
            let rc = self.rand.random_range_int(0, 3, EmNone, "");
            self.random_and_expand(
                if m == 1 { RockGray } else { RockMarble },
                rc,
                3,
                15 + 3 * sc,
                0,
                3,
                CType::NoBorn,
                CType::NoBorn,
            );
            m += 1;
        }
        for t in [IronOre, CopperOre, SilverOre] {
            let (a, b, c) = (
                self.rand.random_range_int(0, sc + 1, EmNone, ""),
                self.rand.random_range_int(1, sc + 1, EmNone, ""),
                self.rand.random_range_int(1, 4, EmNone, ""),
            );
            self.random_and_expand(t, a, b, 13 + sc * c, 0, 3, CType::NoBorn, CType::NoBorn);
        }
        let (rb1, rb2) = (self.rand.random_range_int(2, sc + 1, EmNone, ""), sc + 1);
        self.random_and_expand(
            RockBrown,
            rb1,
            rb2,
            13 + sc * 4,
            if skip_opt { 0 } else { 5 },
            3,
            CType::NoBorn,
            CType::NoBorn,
        );
        for i in 1..3 {
            self.out_line(
                if i == 1 { RockGray } else { RockMarble },
                RockBrown,
                1,
                50 + 12 * sc,
                0,
                CType::NoBorn,
            );
        }
        for t in [IronOre, CopperOre, SilverOre] {
            self.out_line(t, RockBrown, 1, 50 + 12 * sc, 0, CType::NoBorn);
        }
        for t in [IronOre, CopperOre, SilverOre] {
            let r = self.rand.random_range_int(1, 2, EmNone, "");
            self.out_line(t, RockBrown, r, 8 + 8 * sc, 0, CType::NoBorn);
        }
        for j in 0..3 {
            let t = match j {
                1 => RockGray,
                2 => RockMarble,
                _ => RockBrown,
            };
            let r = self.rand.random_range_int(1, sc, EmNone, "");
            self.out_line(t, RockBrown, r, 8 + 8 * sc, 0, CType::NoBorn);
        }
    }
    pub fn step4_post_gpu_expansion(&mut self) {
        for t in [RockBrown, IronOre, SilverOre, CopperOre, RockBrown] {
            self.out_line(t, StoneLand, 1, 30, 0, CType::CheckCon2);
        }
        self.out_line(StoneLand, StoneLand, 1, 5, 0, CType::CheckCon);
    }
    pub fn step5_post_gpu_expansion(&mut self) {
        let sc = 1.max(self.w / 64);
        self.random_and_expand(LingSoil, sc, 6, 33, 5, 3, CType::CheckCon, CType::CheckCon);
    }

    // CPU 的完整纯序直击
    pub fn make_map(&mut self) {
        self.step1_build_base_topology();
        self.optimize(FertileSoil, 2, 4, 1, CType::AllTrue);
        self.step2_post_gpu_expansion(false);
        self.optimize(ShallowWater, 2, 6, 1, CType::CheckCon);
        self.step3_post_gpu_expansion(false);
        self.optimize(RockBrown, 2, 9, 1, CType::NoBorn);
        self.step4_post_gpu_expansion();
        self.optimize(StoneLand, 2, 9, 1, CType::CheckCon);
        self.step5_post_gpu_expansion();
        self.finalize_grid();
    }
}
