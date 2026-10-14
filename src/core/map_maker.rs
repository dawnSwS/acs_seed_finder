// Bitboards mirror the game's integer coordinates and preserve its traversal
// order. Dimensions are validated once; each layer has the exact word count.
// Casts deliberately retain C# integer/float behavior for seed compatibility.
#![allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::indexing_slicing,
    clippy::many_single_char_names
)]

use crate::core::{
    constants::{get_base_around_50, CType},
    rng::DotNetRandom,
    terrain::Terrain::{
        self, BornLine, BornSpace, CopperOre, DDepthWater, DepthWater, FertileSoil, IronOre,
        LingSoil, Mud, Null, RockBrown, RockGray, RockMarble, ShallowWater, SilverOre, Soil,
        StoneLand, Tmp1, Tmp2, WetLand,
    },
};

#[path = "map_outline.rs"]
mod map_outline;

/// The generator accepts bounded rectangular grids; the game normally uses
/// square grids with sides 96, 128, or 192.
pub const MAX_MAP_DIMENSION: i32 = 1024;

/// Which point in the original generation pipeline is being scored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScoringStage {
    /// Immediately after the six initial `LingSoil` expansion passes.
    RawLing6,
    /// The complete map, including soil and default environment processing.
    #[default]
    Postprocessed,
}

/// The scalar kernel remains available for exact differential validation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GenerationKernel {
    Scalar,
    Row4,
    /// Whole-word interval expansion with exact ordered random consumption.
    /// Indexed `OutLine` remains enabled; the selector name is kept for callers.
    Frontier,
    /// Exact target-wise `OutLine` evaluation, with Row4 growth.
    /// Unsupported dimensions or exhausted indexed RNG tables use Row4's
    /// sequential `OutLine` implementation.
    #[default]
    IndexedOutline,
}

/// An owned diagnostic snapshot, including temporary layers and the full RNG.
/// Taking this snapshot is intentionally outside the scanner's hot path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapStateSnapshot {
    pub width: i32,
    pub height: i32,
    pub bitboards: [Vec<u32>; 20],
    pub dirs: Vec<i32>,
    pub rng: DotNetRandom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapSizeError {
    pub width: i32,
    pub height: i32,
}

impl std::fmt::Display for MapSizeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "map dimensions {} × {} must each be in 1..={MAX_MAP_DIMENSION}",
            self.width, self.height
        )
    }
}

impl std::error::Error for MapSizeError {}

pub struct MapMaker {
    w: i32,
    h: i32,
    pub grid: Vec<Terrain>,
    rand: DotNetRandom,
    dirs: Vec<i32>,
    bb: [Vec<u32>; 20],
    kernel: GenerationKernel,
    row4_eligible: Vec<u32>,
    outline_workspace: Option<map_outline::OutlineWorkspace>,
    // Scanner-only quotient state after the two NoLing protection outlines.
    scan_monotone: bool,
    scan_early_ground: bool,
    scan_base: Vec<u32>,
    scan_ling: Vec<u32>,
}

#[derive(Clone, Copy)]
struct ExpansionRule {
    terrain: Terrain,
    level: i32,
    condition: CType,
}

/// Ordered random threshold bits with bounded lookahead. Checkpoints commit
/// exactly the consumed prefix, including when the final refill is unused.
struct ThresholdFifo {
    work: DotNetRandom,
    checkpoint: DotNetRandom,
    previous_checkpoint: DotNetRandom,
    checkpoint_offset: usize,
    previous_offset: usize,
    prefetched: usize,
    consumed: usize,
    bits: u64,
    available: u32,
    level: i32,
}

impl ThresholdFifo {
    fn new(random: &DotNetRandom, level: i32) -> Self {
        Self {
            work: random.clone(),
            checkpoint: random.clone(),
            previous_checkpoint: random.clone(),
            checkpoint_offset: 0,
            previous_offset: 0,
            prefetched: 0,
            consumed: 0,
            bits: 0,
            available: 0,
            level,
        }
    }

    #[inline]
    fn peek(&mut self, required: u32) -> u64 {
        if self.available < required {
            self.previous_checkpoint = std::mem::replace(&mut self.checkpoint, self.work.clone());
            self.previous_offset = self.checkpoint_offset;
            self.checkpoint_offset = self.prefetched;
            self.bits |= u64::from(self.work.next_threshold_mask_32(self.level)) << self.available;
            self.prefetched += 32;
            self.available += 32;
        }
        self.bits
    }

    #[inline]
    fn consume(&mut self, count: u32) {
        self.bits >>= count;
        self.available -= count;
        self.consumed += count as usize;
    }

    #[inline]
    fn next(&mut self) -> bool {
        let success = self.peek(1) & 1 != 0;
        self.consume(1);
        success
    }

    fn finish(self) -> DotNetRandom {
        // Row4 can refill with three old bits; the whole-word kernel with up to 31.
        // The newest checkpoint can be ahead of consumption; use its predecessor.
        let (mut committed, start) = if self.consumed >= self.checkpoint_offset {
            (self.checkpoint, self.checkpoint_offset)
        } else {
            (self.previous_checkpoint, self.previous_offset)
        };
        debug_assert!(self.consumed - start <= 32);
        committed.advance(self.consumed - start);
        committed
    }
}

impl MapMaker {
    pub fn validate_dimensions(w: i32, h: i32) -> Result<(), MapSizeError> {
        if !(1..=MAX_MAP_DIMENSION).contains(&w) || !(1..=MAX_MAP_DIMENSION).contains(&h) {
            return Err(MapSizeError {
                width: w,
                height: h,
            });
        }
        Ok(())
    }

    pub fn try_new(seed: i32, w: i32, h: i32) -> Result<Self, MapSizeError> {
        Self::validate_dimensions(w, h)?;
        let count = (w * h) as usize;
        let words = ((w + 31) / 32 * h) as usize;
        Ok(Self {
            w,
            h,
            rand: DotNetRandom::new(seed),
            grid: vec![Null; count],
            dirs: Vec::with_capacity(2048),
            bb: std::array::from_fn(|_| vec![0; words]),
            kernel: GenerationKernel::default(),
            row4_eligible: vec![0; words],
            outline_workspace: None,
            scan_monotone: false,
            scan_early_ground: false,
            scan_base: vec![0; words],
            scan_ling: vec![0; words],
        })
    }

    pub fn reset(&mut self, s: i32) {
        self.scan_monotone = false;
        self.scan_early_ground = false;
        self.rand = DotNetRandom::new(s);
        self.dirs.clear();
        self.bb.iter_mut().for_each(|b| b.fill(0));
        self.grid.fill(Null);
    }

    pub fn set_kernel(&mut self, kernel: GenerationKernel) {
        self.kernel = kernel;
    }

    /// Warm the seed-independent geometry and RNG coefficient tables. Kept
    /// explicit so applications and benchmarks can account for setup once.
    pub fn prepare_indexed_outline_tables() {
        map_outline::prepare_geometry();
        let _ = DotNetRandom::new(1).indexed_available(1);
    }

    pub fn diagnostic_state(&self) -> MapStateSnapshot {
        MapStateSnapshot {
            width: self.w,
            height: self.h,
            bitboards: self.bb.clone(),
            dirs: self.dirs.clone(),
            rng: self.rand.clone(),
        }
    }
    #[inline]
    fn p2key_safe(&self, x: i32, y: i32) -> i32 {
        if x < 0 || x >= self.w || y < 0 || y >= self.h {
            -1
        } else {
            y * self.w + x
        }
    }
    #[inline]
    fn is_valid_key(&self, key: i32) -> bool {
        key > 0 && key < self.w * self.h
    }
    #[inline]
    fn set_mask(&mut self, terrain: Terrain, w: usize, m: u32) {
        if self.scan_early_ground {
            debug_assert!(matches!(terrain, Soil | FertileSoil));
            if terrain == FertileSoil {
                self.bb[FertileSoil as usize][w] |= m;
            } else {
                self.bb[FertileSoil as usize][w] &= !m;
            }
            return;
        }
        for (i, layer) in self.bb.iter_mut().enumerate().take(16).skip(1) {
            if i == terrain as usize {
                layer[w] |= m;
            } else {
                layer[w] &= !m;
            }
        }
    }
    // The caller has intersected mask with get_ctype_mask(condition).
    // These successful predicates prove which old terrain layers can contain it.
    #[inline]
    fn set_allowed_mask(&mut self, terrain: Terrain, word: usize, mask: u32, condition: CType) {
        if self.scan_monotone {
            debug_assert_eq!(condition, CType::CheckCon);
            debug_assert!(matches!(terrain, Mud | FertileSoil | WetLand));
            self.bb[terrain as usize][word] |= mask;
            return;
        }
        match condition {
            CType::NoBorn => {
                self.bb[Soil as usize][word] &= !mask;
                self.bb[FertileSoil as usize][word] &= !mask;
            }
            CType::CheckCon => {
                self.bb[Soil as usize][word] &= !mask;
                self.bb[FertileSoil as usize][word] &= !mask;
                self.bb[LingSoil as usize][word] &= !mask;
            }
            CType::OnlySoil => {
                self.bb[Soil as usize][word] &= !mask;
            }
            _ => {
                self.set_mask(terrain, word, mask);
                return;
            }
        }
        self.bb[terrain as usize][word] |= mask;
    }

    #[inline]
    fn get_ctype_mask(&self, w: usize, c: CType) -> u32 {
        if self.scan_monotone {
            debug_assert_eq!(c, CType::CheckCon);
            return self.scan_base[w] & !(self.bb[Mud as usize][w] | self.bb[WetLand as usize][w]);
        }
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
                // The marble exception is deliberately asymmetric in the game.
                !(self.bb[IronOre as usize][w]
                    | self.bb[CopperOre as usize][w]
                    | self.bb[SilverOre as usize][w]
                    | self.bb[RockBrown as usize][w]
                    | self.bb[RockGray as usize][w]
                    | (self.bb[RockMarble as usize][w]
                        & !(self.bb[BornSpace as usize][w] | self.bb[BornLine as usize][w])))
            }
            CType::NoLing => !self.bb[LingSoil as usize][w],
            CType::OnlySoil => self.bb[Soil as usize][w],
        };
        if rem != 0 && w % s == s - 1 {
            base &= (1 << rem) - 1;
        }
        base
    }
    #[inline]
    fn get_grid(&self, key: i32, direction: u8) -> i32 {
        if !(0..self.w * self.h).contains(&key) {
            return -1;
        }
        match direction {
            0 => self.vertical_neighbor(key, self.w),
            1 => self.vertical_neighbor(key, -self.w),
            2 => self.horizontal_neighbor(key, -1),
            3 => self.horizontal_neighbor(key, 1),
            4..=7 => self.diagonal_neighbor(key, direction),
            _ => -1,
        }
    }

    #[inline]
    fn vertical_neighbor(&self, key: i32, offset: i32) -> i32 {
        let neighbor = key + offset;
        if neighbor > 0 && neighbor < self.w * self.h {
            neighbor
        } else {
            -1
        }
    }

    #[inline]
    fn horizontal_neighbor(&self, key: i32, offset: i32) -> i32 {
        let neighbor = key + offset;
        if (0..self.w * self.h).contains(&neighbor) && key / self.w == neighbor / self.w {
            neighbor
        } else {
            -1
        }
    }

    #[inline]
    fn diagonal_neighbor(&self, key: i32, direction: u8) -> i32 {
        let (horizontal, vertical) = [(2, 1), (3, 1), (2, 0), (3, 0)][usize::from(direction - 4)];
        let neighbor = self.get_grid(key, horizontal);
        if neighbor == -1 {
            -1
        } else {
            self.get_grid(neighbor, vertical)
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
        for (t, layer) in self.bb.iter_mut().enumerate().take(16).skip(1) {
            if t == def as usize {
                for (w, word) in layer.iter_mut().enumerate() {
                    *word = if rem != 0 && w % s == s - 1 {
                        (1 << rem) - 1
                    } else {
                        !0
                    };
                }
            } else {
                layer.fill(0);
            }
        }
    }
    fn make_mine_dir(&mut self, fx: i32, fy: i32) {
        let (mut i, mut num) = (0, self.rand.next_range(0, self.h));
        let num2 = num;
        let _ = self.rand.next_range(0, self.h);
        while i < self.w {
            i += if self.rand.next_range(0, 100) < 10 {
                -1
            } else {
                1
            };
            num += self.rand.next_range(
                if num2 > self.h / 2 { -fy } else { -fx },
                if num2 > self.h / 2 { fx } else { fy },
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
            self.rand.next_range(0, self.dirs.len() as i32) as usize,
        );
        for _ in 0..size {
            if num >= self.dirs.len() {
                num = self.rand.next_range(0, self.dirs.len() as i32) as usize;
            }
            let mut k = self.dirs[num];
            num += 1;
            let mut vk = [0; 8];
            let n = self.get_cpu_neighbor(k, &mut vk);
            if n > 0 {
                k = vk[self.rand.next_range(0, n as i32) as usize];
            }
            if k >= 0 && k < self.w * self.h {
                let (nw, bit) = (
                    (k / self.w) as usize * s + (k % self.w) as usize / 32,
                    1 << (k % self.w % 32),
                );
                if (self.get_ctype_mask(nw, ctype) & bit) != 0 {
                    self.set_allowed_mask(def, nw, bit, ctype);
                }
            }
        }
        self.out_line(def, def, w, 4, 0, ctype);
    }
    fn out_line(
        &mut self,
        src: Terrain,
        tgt: Terrain,
        width: i32,
        level: i32,
        mut maximum: i32,
        condition: CType,
    ) {
        // Radius-zero self-outline only revisits cells already of the target
        // type. All ordinary terrain layers are mutually exclusive. It cannot
        // change the map; the source snapshot and random position still matter.
        if src == tgt && width == 0 && maximum == 0 {
            let (layers, temporary) = self.bb.split_at_mut(Tmp1 as usize);
            temporary[0].copy_from_slice(&layers[src as usize]);
            let count = self.bb[src as usize]
                .iter()
                .enumerate()
                .map(|(word, bits)| without_origin(word, *bits).count_ones() as usize)
                .sum();
            self.rand.advance_indexed(count);
            return;
        }
        if matches!(
            self.kernel,
            GenerationKernel::IndexedOutline | GenerationKernel::Frontier
        ) && maximum == 0
            && self.out_line_indexed(src, tgt, width, level, condition)
        {
            return;
        }
        if self.kernel != GenerationKernel::Scalar && maximum == 0 {
            self.out_line_batched(src, tgt, width, level, condition);
            return;
        }
        let (layers, temporary) = self.bb.split_at_mut(Tmp1 as usize);
        temporary[0].copy_from_slice(&layers[src as usize]);
        let limit = ((width * 2 + 1).pow(2)) as usize;
        let stride = ((self.w + 31) / 32) as usize;
        let rule = ExpansionRule {
            terrain: tgt,
            level,
            condition,
        };
        for word in (0..stride * self.h as usize).rev() {
            let mut remaining = without_origin(word, self.bb[Tmp1 as usize][word]);
            while remaining != 0 {
                let bit = remaining.ilog2();
                remaining ^= 1 << bit;
                let center = (
                    ((word % stride) * 32 + bit as usize) as i32,
                    (word / stride) as i32,
                );
                if center.0 < self.w {
                    self.spread_around(center, limit, rule, &mut maximum);
                }
            }
        }
    }

    fn out_line_batched(
        &mut self,
        src: Terrain,
        tgt: Terrain,
        width: i32,
        level: i32,
        condition: CType,
    ) {
        let (layers, temporary) = self.bb.split_at_mut(Tmp1 as usize);
        temporary[0].copy_from_slice(&layers[src as usize]);
        let limit = ((width * 2 + 1).pow(2)) as usize;
        let stride = ((self.w + 31) / 32) as usize;
        let mut random = ThresholdFifo::new(&self.rand, level);
        for word in (0..stride * self.h as usize).rev() {
            let mut remaining = without_origin(word, self.bb[Tmp1 as usize][word]);
            while remaining != 0 {
                let bit = remaining.ilog2();
                remaining ^= 1 << bit;
                let x = ((word % stride) * 32 + bit as usize) as i32;
                let y = (word / stride) as i32;
                if x >= self.w {
                    continue;
                }
                for &(dx, dy) in get_base_around_50().iter().take(limit) {
                    let key = self.p2key_safe(x + dx, y + dy);
                    if key >= 0 && random.next() {
                        self.apply_allowed(tgt, key, condition);
                    }
                }
            }
        }
        self.rand = random.finish();
    }

    fn spread_around(
        &mut self,
        center: (i32, i32),
        limit: usize,
        rule: ExpansionRule,
        maximum: &mut i32,
    ) {
        for &(dx, dy) in get_base_around_50().iter().take(limit) {
            let key = self.p2key_safe(center.0 + dx, center.1 + dy);
            if key < 0 {
                continue;
            }
            if self.rand.next_range(0, 100) > rule.level {
                continue;
            }
            if self.apply_allowed(rule.terrain, key, rule.condition) {
                *maximum -= 1;
                if *maximum == 0 {
                    break;
                }
            }
        }
    }

    #[inline]
    fn bit_position(&self, key: i32) -> (usize, u32) {
        let (x, y) = (key % self.w, key / self.w);
        let stride = ((self.w + 31) / 32) as usize;
        (y as usize * stride + x as usize / 32, 1 << (x % 32))
    }

    #[inline]
    fn apply_allowed(&mut self, terrain: Terrain, key: i32, condition: CType) -> bool {
        let (word, bit) = self.bit_position(key);
        if self.get_ctype_mask(word, condition) & bit == 0 {
            return false;
        }
        self.set_allowed_mask(terrain, word, bit, condition);
        true
    }

    fn optimize(
        &mut self,
        terrain: Terrain,
        minimum: i32,
        maximum: i32,
        passes: i32,
        condition: CType,
    ) {
        // No neighbor count can satisfy an empty interval. This operation has
        // no RNG calls, and its temporary layers are overwritten by their next user.
        if minimum > maximum {
            return;
        }
        for _ in 0..passes {
            for word in 0..self.bb[Tmp2 as usize].len() {
                self.bb[Tmp2 as usize][word] = self.get_ctype_mask(word, condition);
            }
            self.update_optimization_layer(terrain, minimum, maximum);
            self.apply_optimization_layer(terrain, condition);
        }
    }

    fn update_optimization_layer(&mut self, terrain: Terrain, minimum: i32, maximum: i32) {
        let stride = ((self.w + 31) / 32) as usize;
        let height = self.h as usize;
        let (layers, temporary) = self.bb.split_at_mut(Tmp1 as usize);
        let (output, eligible) = temporary.split_at_mut(1);
        let source = &layers[terrain as usize];
        for word in 0..stride * height {
            let allowed = eligible[0][word];
            if allowed == 0 {
                output[0][word] = 0;
                continue;
            }
            let neighbors = neighboring_words(source, word, stride, height);
            output[0][word] =
                optimization_mask(neighbor_counts(neighbors), minimum, maximum) & allowed;
        }
    }

    fn apply_optimization_layer(&mut self, terrain: Terrain, condition: CType) {
        for word in 0..self.bb[Tmp1 as usize].len() {
            let added = self.bb[Tmp1 as usize][word] & !self.bb[terrain as usize][word];
            if added != 0 {
                self.set_allowed_mask(terrain, word, added, condition);
            }
        }
    }

    #[allow(clippy::too_many_arguments)] // Matches the source game's explicit pass parameters.
    fn random_and_expand(
        &mut self,
        terrain: Terrain,
        random_count: i32,
        expansion_count: i32,
        expansion_level: i32,
        optimization_minimum: i32,
        optimization_maximum: i32,
        condition: CType,
        expansion_condition: CType,
    ) {
        self.seed_random_terrain(terrain, random_count, condition);
        let rule = ExpansionRule {
            terrain,
            level: expansion_level,
            condition: expansion_condition,
        };
        for pass in 0..expansion_count {
            self.expand_terrain_pass(rule, pass % 2 != 0);
        }
        if optimization_minimum > 0 {
            self.optimize(
                terrain,
                optimization_minimum,
                optimization_maximum,
                1,
                expansion_condition,
            );
        }
    }

    fn seed_random_terrain(&mut self, terrain: Terrain, count: i32, condition: CType) {
        for _ in 0..count.max(1) {
            let (x, y) = (
                self.rand.next_range(0, self.w),
                self.rand.next_range(0, self.h),
            );
            self.apply_allowed(terrain, y * self.w + x, condition);
        }
    }

    fn expand_terrain_pass(&mut self, rule: ExpansionRule, reverse: bool) {
        if self.kernel == GenerationKernel::Frontier {
            self.expand_terrain_pass_word(rule, reverse);
            return;
        }
        // Partial four-cell blocks use the original path. All normal game
        // dimensions (96, 128, 192) use the row transducer.
        if self.kernel != GenerationKernel::Scalar && self.w % 4 == 0 {
            self.expand_terrain_pass_row4(rule, reverse);
        } else {
            self.expand_terrain_pass_scalar(rule, reverse);
        }
    }

    /// Whole-word interval transducer. Existing active runs consume packed
    /// outcomes; eligible gaps extend only through a run of successful draws.
    fn expand_terrain_pass_word(&mut self, rule: ExpansionRule, reverse: bool) {
        let terrain = rule.terrain as usize;
        let stride = ((self.w + 31) / 32) as usize;
        let words = self.bb[terrain].len();
        for word in 0..words {
            self.row4_eligible[word] =
                self.get_ctype_mask(word, rule.condition) & !self.bb[terrain][word];
        }
        let mut random = ThresholdFifo::new(&self.rand, rule.level);
        for offset in 0..words {
            let word = if reverse { words - 1 - offset } else { offset };
            let mut active = self.bb[terrain][word];
            if active == 0 {
                continue;
            }
            let mut eligible = self.row4_eligible[word];
            if reverse {
                active = active.reverse_bits();
                eligible = eligible.reverse_bits();
            }
            let (mut sources, consumed) =
                word_run_transducer(active, eligible, random.peek(32) as u32);
            random.consume(consumed);
            if reverse {
                sources = sources.reverse_bits();
            }
            if sources != 0 {
                self.paint_row4_sources(rule, word, sources, stride);
            }
        }
        self.rand = random.finish();
    }

    /// Skip outcomes that cannot change the map, but retain their exact place
    /// in the random stream. Writes immediately update unscanned active sources.
    /// Retained as a differential oracle for the production whole-word kernel.
    #[cfg(test)]
    fn expand_terrain_pass_frontier(&mut self, rule: ExpansionRule, reverse: bool) -> bool {
        // Each cell can consume at most one sample. Check both requirements
        // before changing any map or workspace state so fallback is atomic.
        if self.w % 32 != 0 {
            return false;
        }
        let Some(reader) = self.rand.indexed_reader((self.w * self.h) as usize) else {
            return false;
        };
        let terrain = rule.terrain as usize;
        let stride = (self.w / 32) as usize;
        let height = self.h as usize;
        let words = self.bb[terrain].len();
        for word in 0..words {
            // Current predicates only read this cell and fixed birth masks.
            // Unwritten cells retain their entry permission throughout a pass.
            self.row4_eligible[word] =
                self.get_ctype_mask(word, rule.condition) & !self.bb[terrain][word];
        }
        let start = self.rand.draw_position();
        let mut consumed = 0usize;
        let bound = super::rng::threshold_bound(rule.level);
        for offset in 0..words {
            let word = if reverse { words - 1 - offset } else { offset };
            if self.bb[terrain][word] == 0 {
                continue;
            }
            let mut boundary = neighboring_words(&self.row4_eligible, word, stride, height)
                .into_iter()
                .fold(0, |a, b| a | b);
            let mut remaining = !0u32;
            loop {
                let active = self.bb[terrain][word] & remaining;
                let candidates = active & boundary;
                if candidates == 0 {
                    consumed += active.count_ones() as usize;
                    break;
                }
                let bit = if reverse {
                    candidates.ilog2()
                } else {
                    candidates.trailing_zeros()
                };
                let prefix = if reverse {
                    !0u32 << bit
                } else {
                    !0u32 >> (31 - bit)
                };
                // Ineffective sources still draw, even when their outcome is
                // not queried. Newly painted sources ahead remain in `active`.
                consumed += (active & prefix).count_ones() as usize;
                remaining &= !prefix;
                if reader.raw_at(start + consumed as u64 - 1) <= bound {
                    self.paint_frontier_source(rule, word, 1u32 << bit, stride);
                    // Eligibility only shrinks; a failed event needs no refresh.
                    boundary = neighboring_words(&self.row4_eligible, word, stride, height)
                        .into_iter()
                        .fold(0, |a, b| a | b);
                }
            }
        }
        self.finish_frontier_terrain(rule);
        self.rand.advance_indexed(consumed);
        true
    }

    #[cfg(test)]
    #[inline]
    fn finish_frontier_terrain(&mut self, rule: ExpansionRule) {
        // Main terrain layers are disjoint at entry. Old target cells need no
        // clearing; clearing at every final target also covers all new writes.
        // Birth and temporary layers keep their original complete-map meaning.
        for word in 0..self.bb[rule.terrain as usize].len() {
            let mask = self.bb[rule.terrain as usize][word];
            if mask != 0 {
                self.set_allowed_mask(rule.terrain, word, mask, rule.condition);
            }
        }
    }

    #[cfg(test)]
    #[inline]
    fn paint_frontier_mask(
        &mut self,
        rule: ExpansionRule,
        word: usize,
        mut mask: u32,
        vertical: bool,
    ) {
        // Match the game's directed key-zero edges, including horizontal writes.
        if vertical && word == 0 {
            mask &= !1;
        }
        mask &= self.row4_eligible[word];
        self.row4_eligible[word] &= !mask;
        self.bb[rule.terrain as usize][word] |= mask;
    }

    #[cfg(test)]
    #[inline]
    fn paint_frontier_source(
        &mut self,
        rule: ExpansionRule,
        word: usize,
        source: u32,
        stride: usize,
    ) {
        let horizontal = (source << 1) | (source >> 1);
        let vertical = source | horizontal;
        let left = if source & 1 != 0 { 1 << 31 } else { 0 };
        let right = source >> 31;
        let column = word % stride;
        let row = word / stride;
        self.paint_frontier_mask(rule, word, horizontal, false);
        if column > 0 && left != 0 {
            self.paint_frontier_mask(rule, word - 1, left, false);
        }
        if column + 1 < stride && right != 0 {
            self.paint_frontier_mask(rule, word + 1, right, false);
        }
        if row > 0 {
            self.paint_frontier_mask(rule, word - stride, vertical, true);
            if column > 0 && left != 0 {
                self.paint_frontier_mask(rule, word - stride - 1, left, true);
            }
            if column + 1 < stride && right != 0 {
                self.paint_frontier_mask(rule, word - stride + 1, right, true);
            }
        }
        if row + 1 < self.h as usize {
            self.paint_frontier_mask(rule, word + stride, vertical, true);
            if column > 0 && left != 0 {
                self.paint_frontier_mask(rule, word + stride - 1, left, true);
            }
            if column + 1 < stride && right != 0 {
                self.paint_frontier_mask(rule, word + stride + 1, right, true);
            }
        }
    }

    fn expand_terrain_pass_scalar(&mut self, rule: ExpansionRule, reverse: bool) {
        let words = self.bb[rule.terrain as usize].len();
        for word_offset in 0..words {
            let word = if reverse {
                words - 1 - word_offset
            } else {
                word_offset
            };
            for bit_offset in 0..32 {
                let bit = if reverse { 31 - bit_offset } else { bit_offset };
                self.expand_terrain_cell(word, bit, rule);
            }
        }
    }

    fn expand_terrain_pass_row4(&mut self, rule: ExpansionRule, reverse: bool) {
        let terrain = rule.terrain as usize;
        let stride = ((self.w + 31) / 32) as usize;
        let words = self.bb[terrain].len();
        for word in 0..words {
            // Only the destination changes, so permission for initially
            // non-target cells is fixed throughout this expansion pass.
            self.row4_eligible[word] =
                self.get_ctype_mask(word, rule.condition) & !self.bb[terrain][word];
        }

        let mut random = ThresholdFifo::new(&self.rand, rule.level);

        for word_offset in 0..words {
            let word = if reverse {
                words - 1 - word_offset
            } else {
                word_offset
            };
            if self.bb[terrain][word] == 0 {
                continue;
            }
            for block in 0..8 {
                let shift = if reverse { 28 - block * 4 } else { block * 4 };
                let active = ((self.bb[terrain][word] >> shift) & 15) as usize;
                if active == 0 {
                    continue;
                }
                let eligible = ((self.row4_eligible[word] >> shift) & 15) as usize;
                let outcomes = random.peek(4) & 15;
                let result = ROW4_TABLE[row4_table_index(active, eligible, outcomes, reverse)];
                let used = u32::from(result >> 4);
                random.consume(used);
                let mut sources = result & 15;
                if sources == 0 {
                    continue;
                }
                if reverse {
                    sources = REVERSED_NIBBLE[usize::from(sources)];
                }
                self.paint_row4_sources(rule, word, u32::from(sources) << shift, stride);
            }
        }

        self.rand = random.finish();
    }

    #[inline]
    fn paint_expansion_mask(
        &mut self,
        rule: ExpansionRule,
        word: usize,
        mut mask: u32,
        vertical: bool,
    ) {
        // The source game's vertical and diagonal edges cannot TARGET key 0;
        // horizontal edges may target it.
        if vertical && word == 0 {
            mask &= !1;
        }
        mask &= self.row4_eligible[word] & !self.bb[rule.terrain as usize][word];
        if mask != 0 {
            self.set_allowed_mask(rule.terrain, word, mask, rule.condition);
        }
    }

    #[inline]
    fn paint_row4_sources(
        &mut self,
        rule: ExpansionRule,
        word: usize,
        sources: u32,
        stride: usize,
    ) {
        let horizontal = (sources << 1) | (sources >> 1);
        let vertical = sources | horizontal;
        let left = if sources & 1 != 0 { 1 << 31 } else { 0 };
        let right = sources >> 31;
        let column = word % stride;
        let row = word / stride;
        self.paint_expansion_mask(rule, word, horizontal, false);
        if column > 0 && left != 0 {
            self.paint_expansion_mask(rule, word - 1, left, false);
        }
        if column + 1 < stride && right != 0 {
            self.paint_expansion_mask(rule, word + 1, right, false);
        }
        if row > 0 {
            self.paint_expansion_mask(rule, word - stride, vertical, true);
            if column > 0 && left != 0 {
                self.paint_expansion_mask(rule, word - stride - 1, left, true);
            }
            if column + 1 < stride && right != 0 {
                self.paint_expansion_mask(rule, word - stride + 1, right, true);
            }
        }
        if row + 1 < self.h as usize {
            self.paint_expansion_mask(rule, word + stride, vertical, true);
            if column > 0 && left != 0 {
                self.paint_expansion_mask(rule, word + stride - 1, left, true);
            }
            if column + 1 < stride && right != 0 {
                self.paint_expansion_mask(rule, word + stride + 1, right, true);
            }
        }
    }

    fn expand_terrain_cell(&mut self, word: usize, bit: usize, rule: ExpansionRule) {
        let stride = ((self.w + 31) / 32) as usize;
        let x = word % stride * 32 + bit;
        if self.bb[rule.terrain as usize][word] & (1 << bit) == 0 || x >= self.w as usize {
            return;
        }
        if self.rand.next_range(0, 100) > rule.level {
            return;
        }
        let key = (word / stride * self.w as usize + x) as i32;
        let mut neighbors = [0; 8];
        let count = self.get_cpu_neighbor(key, &mut neighbors);
        for &neighbor in neighbors.iter().take(count) {
            self.apply_allowed(rule.terrain, neighbor, rule.condition);
        }
    }

    fn finalize_grid(&mut self) {
        let (s, w) = (((self.w + 31) / 32) as usize, self.w as usize);
        self.grid.fill(Null);
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
                    WetLand,
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
    }

    // Terrain phases follow the supplied game source and share one RNG state.
    fn build_base_topology(&mut self) {
        let (s, _) = (((self.w + 31) / 32) as usize, self.w as usize);
        for _ in 0..3 {
            self.make_mine_dir(2, 3);
        }
        self.fill(Soil);
        let (kx, ky) = (
            self.rand.next_range(self.w / 10 * 4, self.w / 10 * 7),
            self.rand.next_range(self.h / 10 * 6, self.h / 10 * 7),
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
    fn build_water(&mut self) {
        let scale = 1.max(self.w / 64);
        self.snapshot_born_space();
        self.elide_born_fertile_outline_and_clear();
        self.build_born_lines();
        self.random_and_expand(FertileSoil, 20, 4, 30, 5, 3, CType::AllTrue, CType::AllTrue);
        if self.scan_early_ground {
            for word in 0..self.bb[Soil as usize].len() {
                self.bb[Soil as usize][word] &= !self.bb[FertileSoil as usize][word];
            }
            self.scan_early_ground = false;
        }
        self.random_and_expand(
            DDepthWater,
            scale - 1,
            2 * scale - 1,
            13 + 6 * scale,
            5,
            3,
            CType::NoBorn,
            CType::NoBorn,
        );
        self.out_line(DDepthWater, DepthWater, 1, 100, 0, CType::CheckCon);
        self.out_line(DepthWater, DepthWater, 1, 10 + 6 * scale, 0, CType::NoBorn);
        self.out_line(
            DepthWater,
            ShallowWater,
            4,
            50 + 12 * scale,
            0,
            CType::CheckCon,
        );
    }
    /// The only ordinary terrains at this point of generation are Soil and
    /// `FertileSoil`. The guaranteed radius-two dilation is immediately cleared
    /// back to Soil at every nonzero key, so every nonzero dilation write is
    /// dead. Key zero is the sole exception to the clear operation.
    fn elide_born_fertile_outline_and_clear(&mut self) {
        let (layers, temporary) = self.bb.split_at_mut(Tmp1 as usize);
        temporary[0].copy_from_slice(&layers[FertileSoil as usize]);
        let stride = ((self.w + 31) / 32) as usize;
        let mut count = 0usize;
        let mut reaches_origin = false;
        for word in 0..self.bb[FertileSoil as usize].len() {
            let y = (word / stride) as i32;
            let span_y = ((y + 2).min(self.h - 1) - (y - 2).max(0) + 1) as usize;
            let mut bits = without_origin(word, self.bb[FertileSoil as usize][word]);
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let x = (word % stride * 32 + bit) as i32;
                let span_x = ((x + 2).min(self.w - 1) - (x - 2).max(0) + 1) as usize;
                count += span_x * span_y;
                reaches_origin |= x <= 2 && y <= 2;
            }
        }
        self.rand.advance_indexed(count);
        self.clear_born_fertile_soil();
        if reaches_origin {
            self.set_mask(FertileSoil, 0, 1);
        }
    }

    fn snapshot_born_space(&mut self) {
        for word in 0..self.bb[BornSpace as usize].len() {
            self.bb[BornSpace as usize][word] =
                without_origin(word, self.bb[FertileSoil as usize][word]);
        }
    }

    fn clear_born_fertile_soil(&mut self) {
        for word in 0..self.bb[FertileSoil as usize].len() {
            let mask = without_origin(word, self.bb[FertileSoil as usize][word]);
            if mask != 0 {
                self.set_mask(Soil, word, mask);
            }
        }
    }

    fn build_born_lines(&mut self) {
        self.bb[BornLine as usize].fill(0);
        for side in 0..4 {
            self.add_born_line(side);
        }
    }

    fn add_born_line(&mut self, side: u8) {
        let start = self
            .rand
            .next_range((self.w as f32 * 0.3) as i32, (self.w as f32 * 0.6) as i32);
        let mut offset = 0;
        // The loop length is sampled on every condition check in the game.
        while offset < self.rand.next_range(5, 15) {
            let key = self.border_key(side, start + offset);
            if key >= 0 {
                let (word, bit) = self.bit_position(key);
                self.bb[BornLine as usize][word] |= bit;
            }
            offset += 1;
        }
    }

    fn border_key(&self, side: u8, offset: i32) -> i32 {
        match side {
            0 => self.p2key_safe(0, offset),
            1 => self.p2key_safe(self.w - 1, offset),
            2 => self.p2key_safe(offset, 0),
            _ => self.p2key_safe(offset, self.h - 1),
        }
    }

    fn build_mines(&mut self) {
        let scale = 1.max(self.w / 64);
        self.random_and_expand(ShallowWater, 3, 3, 20, 5, 3, CType::NoBorn, CType::NoBorn);
        self.out_line(ShallowWater, Mud, 4, 90, 0, CType::CheckCon);
        self.build_mine_lines(scale);
        self.expand_mines(scale);
        self.build_mine_borders(scale);
    }

    fn build_mine_lines(&mut self, sc: i32) {
        let mut idx1 = 0;
        while idx1 < self.rand.next_range(sc, sc + 2) {
            let (sz, x) = (
                self.rand.next_range(0, sc),
                self.rand.next_range(5 + sc, 10 + sc),
            );
            self.random_line_from_mine_dir(sz, x, IronOre, CType::NoBorn);
            idx1 += 1;
        }
        for t in [CopperOre, SilverOre] {
            for _ in 0..=sc {
                let (wr, x) = (
                    self.rand.next_range(0, 1),
                    self.rand.next_range(3 + sc, 5 + sc),
                );
                self.random_line_from_mine_dir(wr, x, t, CType::NoBorn);
            }
        }
        let mut idx4 = 0;
        while idx4 < self.rand.next_range(1, 3) {
            let (wr, x, st) = (
                self.rand.next_range(0, 1),
                self.rand.next_range(8, 16),
                if self.rand.next_range(1, 3) == 1 {
                    RockGray
                } else {
                    RockMarble
                },
            );
            self.random_line_from_mine_dir(wr, x, st, CType::NoBorn);
            idx4 += 1;
        }
    }

    fn expand_mines(&mut self, sc: i32) {
        let mut m = 1;
        while m < self.rand.next_range(1, 3) {
            let rc = self.rand.next_range(0, 3);
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
                self.rand.next_range(0, sc + 1),
                self.rand.next_range(1, sc + 1),
                self.rand.next_range(1, 4),
            );
            self.random_and_expand(t, a, b, 13 + sc * c, 0, 3, CType::NoBorn, CType::NoBorn);
        }
        let (rb1, rb2) = (self.rand.next_range(2, sc + 1), sc + 1);
        self.random_and_expand(
            RockBrown,
            rb1,
            rb2,
            13 + sc * 4,
            0,
            3,
            CType::NoBorn,
            CType::NoBorn,
        );
    }

    fn build_mine_borders(&mut self, sc: i32) {
        let fused = matches!(
            self.kernel,
            GenerationKernel::IndexedOutline | GenerationKernel::Frontier
        ) && self.out_line_group_indexed(
            &[
                (RockGray, 50 + 12 * sc),
                (RockMarble, 50 + 12 * sc),
                (IronOre, 50 + 12 * sc),
                (CopperOre, 50 + 12 * sc),
                (SilverOre, 50 + 12 * sc),
                (IronOre, 8 + 8 * sc),
                (CopperOre, 8 + 8 * sc),
                (SilverOre, 8 + 8 * sc),
            ],
            RockBrown,
            CType::NoBorn,
        );
        if !fused {
            for terrain in [RockGray, RockMarble] {
                self.out_line(terrain, RockBrown, 1, 50 + 12 * sc, 0, CType::NoBorn);
            }
            for t in [IronOre, CopperOre, SilverOre] {
                self.out_line(t, RockBrown, 1, 50 + 12 * sc, 0, CType::NoBorn);
            }
            for t in [IronOre, CopperOre, SilverOre] {
                // This singleton range consumes no RNG draw in the game.
                let r = self.rand.next_range(1, 2);
                self.out_line(t, RockBrown, r, 8 + 8 * sc, 0, CType::NoBorn);
            }
        }
        for t in [RockBrown, RockGray, RockMarble] {
            let r = self.rand.next_range(1, sc);
            self.out_line(t, RockBrown, r, 8 + 8 * sc, 0, CType::NoBorn);
        }
    }

    fn build_stone_land(&mut self) {
        let fused = matches!(
            self.kernel,
            GenerationKernel::IndexedOutline | GenerationKernel::Frontier
        ) && self.out_line_group_indexed(
            &[
                (RockBrown, 30),
                (IronOre, 30),
                (SilverOre, 30),
                (CopperOre, 30),
                (RockBrown, 30),
            ],
            StoneLand,
            CType::CheckCon2,
        );
        if !fused {
            for t in [RockBrown, IronOre, SilverOre, CopperOre, RockBrown] {
                self.out_line(t, StoneLand, 1, 30, 0, CType::CheckCon2);
            }
        }
        self.out_line(StoneLand, StoneLand, 1, 5, 0, CType::CheckCon);
    }
    fn build_ling_soil(&mut self) {
        let sc = 1.max(self.w / 64);
        self.random_and_expand(LingSoil, sc, 6, 33, 5, 3, CType::CheckCon, CType::CheckCon);
    }

    fn postprocess_soils(&mut self) {
        self.postprocess_ling_overwrites();
        self.out_line(FertileSoil, WetLand, 2, 2, 0, CType::OnlySoil);
        self.optimize(WetLand, 2, 9, 1, CType::OnlySoil);
    }

    fn postprocess_ling_overwrites(&mut self) {
        self.postprocess_ling_overwrites_impl(false);
    }

    fn postprocess_ling_overwrites_impl(&mut self, monotone: bool) {
        // Subsequent passes can replace LingSoil because CheckCon includes it.
        // RawLing6 intentionally scores before these passes; Postprocessed
        // retains the complete terrain-generation behavior.
        self.out_line(LingSoil, WetLand, 1, 100, 0, CType::NoLing);
        self.out_line(WetLand, WetLand, 1, 90, 0, CType::NoLing);
        if monotone {
            for word in 0..self.scan_base.len() {
                self.scan_base[word] = self.bb[Soil as usize][word]
                    | self.bb[FertileSoil as usize][word]
                    | self.bb[LingSoil as usize][word];
                self.scan_ling[word] = self.bb[LingSoil as usize][word];
            }
            self.scan_monotone = true;
        }
        self.out_line(WetLand, WetLand, 1, 5, 0, CType::CheckCon);
        self.optimize(WetLand, 2, 9, 1, CType::CheckCon);
        self.out_line(Mud, Mud, 1, 10, 0, CType::CheckCon);
        self.optimize(Mud, 2, 9, 1, CType::CheckCon);
        for terrain in [Mud, FertileSoil, WetLand] {
            if monotone && terrain == FertileSoil {
                // Earlier absorbing writes may have left stale fertile bits.
                // This is the sole remaining stage which reads FertileSoil as
                // a source. Clean it once; during its own expansion M/W freeze.
                for word in 0..self.scan_base.len() {
                    self.bb[FertileSoil as usize][word] &=
                        !(self.bb[Mud as usize][word] | self.bb[WetLand as usize][word]);
                }
            }
            self.random_and_expand(terrain, 5, 3, 20, 5, 3, CType::CheckCon, CType::CheckCon);
        }
    }

    fn finish_default_environment(&mut self) {
        // Default ReplaceEnv converts fertile soil near spirit soil to mud.
        // The first 25 spiral offsets cover the complete radius-two square.
        // Snapshot horizontal coverage before applying any writes, then OR the
        // five neighboring rows. This preserves _FixEnvTerrain's snapshot while
        // replacing per-cell coordinate divisions and 25-neighbor searches.
        // Reuse non-observable expansion scratch; Tmp1/Tmp2 remain unchanged.
        let stride = ((self.w + 31) / 32) as usize;
        let height = self.h as usize;
        for (source, coverage) in self.bb[LingSoil as usize]
            .chunks_exact(stride)
            .zip(self.row4_eligible.chunks_exact_mut(stride))
        {
            for column in 0..stride {
                let center = source[column];
                let left = if column > 0 { source[column - 1] } else { 0 };
                let right = if column + 1 < stride {
                    source[column + 1]
                } else {
                    0
                };
                coverage[column] = center
                    | (center << 1)
                    | (center << 2)
                    | (center >> 1)
                    | (center >> 2)
                    | (left >> 31)
                    | (left >> 30)
                    | (right << 31)
                    | (right << 30);
            }
        }
        for row in 0..height {
            for column in 0..stride {
                let word = row * stride + column;
                // Target key zero is excluded, but spirit soil at key zero
                // still contributes to neighboring targets in this operation.
                let fertile = without_origin(word, self.bb[FertileSoil as usize][word]);
                if fertile == 0 {
                    continue;
                }
                let mut nearby = 0;
                for source_row in row.saturating_sub(2)..=(row + 2).min(height - 1) {
                    nearby |= self.row4_eligible[source_row * stride + column];
                }
                let converted = fertile & nearby;
                if converted != 0 {
                    self.set_mask(Mud, word, converted);
                }
            }
        }
    }

    #[cfg(test)]
    fn finish_default_environment_scalar(&mut self) {
        // Retain the original cell-wise algorithm as an independent oracle.
        let stride = ((self.w + 31) / 32) as usize;
        let mut converted = Vec::new();
        for key in 1..self.w * self.h {
            let (x, y) = (key % self.w, key / self.w);
            let word = y as usize * stride + x as usize / 32;
            let bit = 1 << (x % 32);
            if self.bb[FertileSoil as usize][word] & bit == 0 {
                continue;
            }
            let near_ling = get_base_around_50().iter().take(25).any(|&(dx, dy)| {
                let neighbor = self.p2key_safe(x + dx, y + dy);
                if neighbor < 0 {
                    return false;
                }
                let (nx, ny) = (neighbor % self.w, neighbor / self.w);
                self.bb[LingSoil as usize][ny as usize * stride + nx as usize / 32]
                    & (1 << (nx % 32))
                    != 0
            });
            if near_ling {
                converted.push(key);
            }
        }
        for key in converted {
            let (x, y) = (key % self.w, key / self.w);
            self.set_mask(Mud, y as usize * stride + x as usize / 32, 1 << (x % 32));
        }
    }

    pub fn count_ling_soil(&self) -> usize {
        self.grid
            .iter()
            .filter(|&&terrain| terrain == LingSoil)
            .count()
    }

    /// Generate default, unmodified game terrain in its original traversal order.
    pub fn make_map(&mut self) {
        self.generate_terrain(ScoringStage::Postprocessed);
        self.finalize_grid();
    }

    /// Generate and export the map at the explicitly selected scoring stage.
    /// As with `make_map`, call `reset(seed)` before reusing this generator.
    pub fn make_map_at_stage(&mut self, stage: ScoringStage) {
        self.generate_terrain(stage);
        self.finalize_grid();
    }

    /// Generate and score bitboards without building the display grid.
    /// `grid` is not updated. Call `reset(seed)` before each reused generation.
    pub fn score(&mut self, stage: ScoringStage) -> usize {
        self.generate_terrain(stage);
        self.count_ling_bitboard()
    }

    /// Scanner-only evaluation: finish the selected score before filtering.
    /// Omits the non-scoring environment tail and compresses postprocessed state
    /// into overlapping scanner-only bitboards. Reset before reuse; never export.
    pub(crate) fn score_for_scan(&mut self, stage: ScoringStage, minimum: usize) -> Option<usize> {
        self.scan_early_ground = true;
        self.generate_ling_prefix();
        let count = if stage == ScoringStage::Postprocessed {
            self.postprocess_ling_overwrites_impl(true);
            self.scan_ling
                .iter()
                .enumerate()
                .map(|(word, ling)| {
                    (ling
                        & !(self.bb[FertileSoil as usize][word]
                            | self.bb[Mud as usize][word]
                            | self.bb[WetLand as usize][word]))
                        .count_ones() as usize
                })
                .sum()
        } else {
            self.count_ling_bitboard()
        };
        (count >= minimum).then_some(count)
    }

    fn count_ling_bitboard(&self) -> usize {
        self.bb[LingSoil as usize]
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    fn generate_terrain(&mut self, stage: ScoringStage) {
        self.generate_ling_prefix();
        if stage == ScoringStage::Postprocessed {
            self.postprocess_soils();
            self.finish_default_environment();
        }
    }

    fn generate_ling_prefix(&mut self) {
        self.build_base_topology();
        self.optimize(FertileSoil, 2, 4, 1, CType::AllTrue);
        self.build_water();
        self.optimize(ShallowWater, 2, 6, 1, CType::CheckCon);
        self.build_mines();
        self.optimize(RockBrown, 2, 9, 1, CType::NoBorn);
        self.build_stone_land();
        self.optimize(StoneLand, 2, 9, 1, CType::CheckCon);
        self.build_ling_soil();
    }
}

/// Scan-direction coordinates. `active` is the current target mask at entry;
/// only the previous successful source can activate a later cell in this word.
/// Each original active run is read once. A following eligible gap is traversed
/// up to its first failed draw, its first forbidden cell, or the next active run.
#[inline]
fn word_run_transducer(mut active: u32, eligible: u32, outcomes: u32) -> (u32, u32) {
    let mut sources = 0u32;
    let mut consumed = 0u32;
    while active != 0 {
        let start = active.trailing_zeros();
        let len = (active >> start).trailing_ones();
        let run_mask = if len == 32 { !0 } else { (1u32 << len) - 1 };
        let draws = outcomes >> consumed;
        let successes = draws & run_mask;
        sources |= successes << start;
        consumed += len;
        active &= !(run_mask << start);
        let next = start + len;
        if next == 32 || successes & (1u32 << (len - 1)) == 0 {
            continue;
        }
        let gap = ((eligible & !active) >> next)
            .trailing_ones()
            .min(32 - next);
        if gap == 0 {
            continue;
        }
        // `next < 32` implies `consumed < 32`: every consumed draw belongs
        // to an earlier cell. Read only the prefix that can become active.
        let success_len = (outcomes >> consumed).trailing_ones().min(gap);
        if success_len > 0 {
            sources |= ((1u32 << success_len) - 1) << next;
        }
        consumed += (success_len + 1).min(gap);
    }
    (sources, consumed)
}

const REVERSED_NIBBLE: [u8; 16] = [0, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];

#[inline]
fn row4_table_index(mut active: usize, mut eligible: usize, outcomes: u64, reverse: bool) -> usize {
    if reverse {
        active = usize::from(REVERSED_NIBBLE[active]);
        eligible = usize::from(REVERSED_NIBBLE[eligible]);
    }
    active | (eligible << 4) | ((outcomes as usize) << 8)
}

/// A four-cell transducer in scan order: A=current target, E=eligible,
/// R=the next four random threshold results. Output packs successful sources
/// in bits 0..3 and the actual number of consumed random values in bits 4..6.
const ROW4_TABLE: [u8; 4096] = build_row4_table();

const fn build_row4_table() -> [u8; 4096] {
    let mut table = [0; 4096];
    let mut index = 0;
    while index < table.len() {
        let active = index & 15;
        let eligible = (index >> 4) & 15;
        let random = index >> 8;
        let mut sources = 0;
        let mut consumed = 0;
        let mut cell = 0;
        while cell < 4 {
            let live = active & (1 << cell) != 0
                || (cell > 0 && eligible & (1 << cell) != 0 && sources & (1 << (cell - 1)) != 0);
            if live {
                if random & (1 << consumed) != 0 {
                    sources |= 1 << cell;
                }
                consumed += 1;
            }
            cell += 1;
        }
        table[index] = (sources | (consumed << 4)) as u8;
        index += 1;
    }
    table
}

#[inline]
fn without_origin(word: usize, mask: u32) -> u32 {
    if word == 0 {
        mask & !1
    } else {
        mask
    }
}

#[inline]
fn shifted_row(layer: &[u32], word: usize, stride: usize, exclude_origin: bool) -> [u32; 3] {
    let column = word % stride;
    let center = if exclude_origin && column == 0 {
        layer[word] & !1
    } else {
        layer[word]
    };
    let left = if column > 0 {
        (center << 1) | (layer[word - 1] >> 31)
    } else {
        center << 1
    };
    let right = if column < stride - 1 {
        (center >> 1) | (layer[word + 1] << 31)
    } else {
        center >> 1
    };
    [left, center, right]
}

#[inline]
fn neighboring_words(layer: &[u32], word: usize, stride: usize, height: usize) -> [u32; 8] {
    let row = word / stride;
    let [middle_left, _, middle_right] = shifted_row(layer, word, stride, false);
    let [upper_left, upper, upper_right] = if row > 0 {
        shifted_row(layer, word - stride, stride, row == 1)
    } else {
        [0; 3]
    };
    let [lower_left, lower, lower_right] = if row < height - 1 {
        shifted_row(layer, word + stride, stride, false)
    } else {
        [0; 3]
    };
    [
        middle_left,
        middle_right,
        upper_left,
        upper,
        upper_right,
        lower_left,
        lower,
        lower_right,
    ]
}

#[inline]
fn full_adder(first: u32, second: u32, third: u32) -> (u32, u32) {
    let partial = first ^ second;
    (partial ^ third, (first & second) | (partial & third))
}

#[inline]
fn neighbor_counts(neighbors: [u32; 8]) -> [u32; 4] {
    let [ml, mr, ul, uc, ur, dl, dc, dr] = neighbors;
    let (s0, c1) = (ml ^ mr, ml & mr);
    let (s1, c2) = full_adder(ul, uc, ur);
    let (s2, c3) = full_adder(dl, dc, dr);
    let (s01, c11) = full_adder(s0, s1, s2);
    let (s10, c20) = full_adder(c1, c2, c3);
    let (s11, c21) = (s10 ^ c11, s10 & c11);
    let (s20, c30) = (c20 ^ c21, c20 & c21);
    [s01, s11, s20, c30]
}

#[inline]
fn optimization_mask(bits: [u32; 4], minimum: i32, maximum: i32) -> u32 {
    let [b0, b1, b2, b3] = bits;
    match (minimum, maximum) {
        (2, 4) => (!b3 & !b2 & b1) | (!b3 & b2 & !b1 & !b0),
        (2, 6) => (!b3 & !b2 & b1) | (!b3 & b2 & !b1) | (!b3 & b2 & b1 & !b0),
        (2, 8 | 9) => b3 | b2 | b1,
        _ => count_range_mask(bits, minimum as u32, maximum as u32),
    }
}

fn count_range_mask(bits: [u32; 4], minimum: u32, maximum: u32) -> u32 {
    let [b0, b1, b2, b3] = bits;
    let mut mask = 0;
    for bit in 0..32 {
        let count = ((b3 >> bit) & 1) * 8
            + ((b2 >> bit) & 1) * 4
            + ((b1 >> bit) & 1) * 2
            + ((b0 >> bit) & 1);
        if (minimum..=maximum).contains(&count) {
            mask |= 1 << bit;
        }
    }
    mask
}

#[cfg(test)]
mod scan_score_tests {
    use super::*;

    fn assert_scan_thresholds(
        candidate: &mut MapMaker,
        seed: i32,
        stage: ScoringStage,
        expected: usize,
        expected_state: &MapStateSnapshot,
        thresholds: &[usize],
    ) {
        let size = candidate.w;
        for &minimum in thresholds {
            for kernel in [GenerationKernel::IndexedOutline, GenerationKernel::Frontier] {
                candidate.set_kernel(kernel);
                candidate.reset(seed);
                assert_eq!(
                    candidate.score_for_scan(stage, minimum),
                    (expected >= minimum).then_some(expected),
                    "size={size}, seed={seed}, stage={stage:?}, minimum={minimum}, kernel={kernel:?}"
                );
                // Even a threshold above the raw count must finish the selected
                // stage; the full-generation score remains the independent oracle.
                // The early quotient has already reconstructed Soil before
                // either observable scoring stage; there is nothing to decode.
                assert!(!candidate.scan_early_ground);
                let mut projected = candidate.diagnostic_state();
                if candidate.scan_monotone {
                    for word in 0..candidate.scan_base.len() {
                        let absorbed =
                            candidate.bb[Mud as usize][word] | candidate.bb[WetLand as usize][word];
                        let ever_fertile = candidate.bb[FertileSoil as usize][word];
                        projected.bitboards[FertileSoil as usize][word] = ever_fertile & !absorbed;
                        projected.bitboards[LingSoil as usize][word] =
                            candidate.scan_ling[word] & !(absorbed | ever_fertile);
                        projected.bitboards[Soil as usize][word] = candidate.scan_base[word]
                            & !(absorbed | ever_fertile | candidate.scan_ling[word]);
                    }
                }
                // Compare the decoded full state, including exact RNG. The
                // scanner's internal quotient deliberately has stale F/S/L.
                assert_eq!(&projected, expected_state);
            }
        }
    }

    #[test]
    fn scan_scores_match_full_generation_at_threshold_boundaries() -> Result<(), MapSizeError> {
        for size in [96, 128, 192] {
            let mut reference = MapMaker::try_new(0, size, size)?;
            let mut candidate = MapMaker::try_new(0, size, size)?;
            for seed in [0, 17_547_620, -12_345, 321_251_673, i32::MAX, i32::MIN] {
                reference.reset(seed);
                let raw = reference.score(ScoringStage::RawLing6);
                let raw_state = reference.diagnostic_state();
                reference.reset(seed);
                let final_count = reference.score(ScoringStage::Postprocessed);
                assert!(final_count <= raw, "size={size}, seed={seed}");
                reference.reset(seed);
                reference.generate_ling_prefix();
                reference.postprocess_ling_overwrites();
                let final_scan_state = reference.diagnostic_state();
                assert_eq!(reference.count_ling_bitboard(), final_count);
                let mut thresholds = vec![
                    0,
                    raw.saturating_sub(1),
                    raw,
                    raw + 1,
                    final_count.saturating_sub(1),
                    final_count,
                    final_count + 1,
                    usize::MAX,
                ];
                thresholds.sort_unstable();
                thresholds.dedup();
                for (stage, expected, state) in [
                    (ScoringStage::RawLing6, raw, &raw_state),
                    (ScoringStage::Postprocessed, final_count, &final_scan_state),
                ] {
                    assert_scan_thresholds(
                        &mut candidate,
                        seed,
                        stage,
                        expected,
                        state,
                        &thresholds,
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod environment_tests {
    use super::*;

    fn assert_matches_scalar(mut reference: MapMaker) -> Result<(), MapSizeError> {
        let mut candidate = MapMaker::try_new(0, reference.w, reference.h)?;
        candidate.bb.clone_from(&reference.bb);
        candidate.dirs.clone_from(&reference.dirs);
        candidate.grid.clone_from(&reference.grid);
        candidate.rand = reference.rand.clone();
        // Previous expansion scratch must have no influence on the result.
        candidate.row4_eligible.fill(0xa5a5_a5a5);
        let random_before = reference.rand.clone();
        reference.finish_default_environment_scalar();
        candidate.finish_default_environment();
        assert_eq!(candidate.diagnostic_state(), reference.diagnostic_state());
        assert_eq!(candidate.grid, reference.grid);
        assert_eq!(candidate.rand, random_before);
        Ok(())
    }

    #[test]
    fn bitboard_environment_matches_scalar_on_rectangles_and_standard_maps(
    ) -> Result<(), MapSizeError> {
        for (width, height) in [
            (1, 1),
            (1, 9),
            (3, 2),
            (7, 35),
            (28, 5),
            (31, 9),
            (32, 7),
            (33, 11),
            (36, 7),
            (65, 9),
            (96, 96),
            (128, 128),
            (192, 192),
        ] {
            for seed in [0, 17_547_620, i32::MAX] {
                let mut map = MapMaker::try_new(seed, width, height)?;
                let mut fixture = DotNetRandom::new(seed);
                map.fill(FertileSoil);
                for key in 0..width * height {
                    let terrain = match fixture.next_range(0, 8) {
                        0 => LingSoil,
                        1 => Soil,
                        2 => Mud,
                        _ => FertileSoil,
                    };
                    let (word, bit) = map.bit_position(key);
                    map.set_mask(terrain, word, bit);
                }
                // The environmental pass must preserve diagnostic scratch,
                // birth masks, mine directions, and the complete RNG state.
                map.bb[Tmp1 as usize].fill(0x1234_5678);
                map.bb[Tmp2 as usize].fill(0x8765_4321);
                map.bb[BornLine as usize].fill(0x55aa_55aa);
                map.bb[BornSpace as usize].fill(0xaa55_aa55);
                map.dirs.extend([0, width * height - 1]);
                map.rand.advance(79);
                assert_matches_scalar(map)?;
            }
        }
        Ok(())
    }

    #[test]
    fn bitboard_environment_clips_edges_and_word_boundaries() -> Result<(), MapSizeError> {
        for (width, height) in [(1, 7), (2, 3), (31, 5), (32, 5), (33, 5), (65, 7)] {
            let mut columns = vec![0, 1, 2, 30, 31, 32, width - 3, width - 2, width - 1];
            columns.retain(|&x| (0..width).contains(&x));
            columns.sort_unstable();
            columns.dedup();
            for x in columns {
                for y in 0..height {
                    let mut map = MapMaker::try_new(1, width, height)?;
                    map.fill(FertileSoil);
                    let (word, bit) = map.bit_position(y * width + x);
                    map.set_mask(LingSoil, word, bit);
                    assert_matches_scalar(map)?;
                }
            }
        }
        Ok(())
    }

    #[test]
    fn environment_keeps_origin_as_a_source_but_never_as_a_target() -> Result<(), MapSizeError> {
        let mut source_origin = MapMaker::try_new(1, 4, 4)?;
        source_origin.fill(FertileSoil);
        source_origin.set_mask(LingSoil, 0, 1);
        source_origin.finish_default_environment();
        assert_ne!(source_origin.bb[Mud as usize][0] & (1 << 2), 0);
        assert_ne!(source_origin.bb[Mud as usize][2] & (1 << 2), 0);
        assert_eq!(source_origin.bb[Mud as usize][0] & (1 << 3), 0);

        let mut target_origin = MapMaker::try_new(1, 4, 4)?;
        target_origin.fill(FertileSoil);
        target_origin.set_mask(LingSoil, 0, 1 << 1);
        target_origin.finish_default_environment();
        assert_ne!(target_origin.bb[FertileSoil as usize][0] & 1, 0);
        assert_eq!(target_origin.bb[Mud as usize][0] & 1, 0);
        Ok(())
    }
}

#[cfg(test)]
mod row4_tests {
    use super::*;

    #[test]
    fn table_matches_all_sequential_four_cell_inputs() {
        for (index, &entry) in ROW4_TABLE.iter().enumerate() {
            let mut active = index & 15;
            let eligible = (index >> 4) & 15;
            let random = index >> 8;
            let mut consumed = 0;
            let mut sources = 0;
            for cell in 0..4 {
                if active & (1 << cell) == 0 {
                    continue;
                }
                let success = random & (1 << consumed) != 0;
                consumed += 1;
                if success {
                    sources |= 1 << cell;
                    if cell > 0 {
                        active |= eligible & (1 << (cell - 1));
                    }
                    if cell < 3 {
                        active |= eligible & (1 << (cell + 1));
                    }
                }
            }
            assert_eq!(
                usize::from(entry),
                sources | (consumed << 4),
                "index={index}"
            );
        }
    }

    #[test]
    fn row4_preserves_every_layer_and_rng_at_both_score_stages() -> Result<(), MapSizeError> {
        // Non-word widths exercise clipped words; non-four-cell widths exercise
        // the explicit scalar fallback. Narrow maps expose key-zero edges.
        for (width, height, seed) in [
            (1, 1, 1),
            (3, 5, 42),
            (4, 3, 123),
            (28, 5, 2_147_483_647),
            (36, 7, 321_251_673),
            (96, 96, 17_547_620),
            (192, 192, 321_251_673),
        ] {
            for stage in [ScoringStage::RawLing6, ScoringStage::Postprocessed] {
                let mut scalar = MapMaker::try_new(seed, width, height)?;
                scalar.set_kernel(GenerationKernel::Scalar);
                let mut row4 = MapMaker::try_new(seed, width, height)?;
                let expected = scalar.score(stage);
                assert_eq!(
                    expected,
                    row4.score(stage),
                    "{width}x{height}, seed={seed}, stage={stage:?}"
                );
                assert_eq!(
                    scalar.diagnostic_state(),
                    row4.diagnostic_state(),
                    "{width}x{height}, seed={seed}, stage={stage:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn original_make_map_still_exports_postprocessed_terrain() -> Result<(), MapSizeError> {
        let seed = 321_251_673;
        let mut raw = MapMaker::try_new(seed, 192, 192)?;
        assert_eq!(raw.score(ScoringStage::RawLing6), 6876);
        let mut score_only = MapMaker::try_new(seed, 192, 192)?;
        let expected = score_only.score(ScoringStage::Postprocessed);
        let mut display = MapMaker::try_new(seed, 192, 192)?;
        display.make_map();
        assert_eq!(expected, display.count_ling_soil());
        assert_eq!(score_only.diagnostic_state(), display.diagnostic_state());
        Ok(())
    }
}

#[cfg(test)]
mod frontier_tests {
    use super::*;

    fn adversarial_state(seed: i32, width: i32, height: i32) -> Result<MapMaker, MapSizeError> {
        let mut fixture = DotNetRandom::new(seed);
        let mut initial = MapMaker::try_new(seed, width, height)?;
        for key in 0..width * height {
            let terrain = [Soil, FertileSoil, LingSoil, WetLand, RockBrown, IronOre]
                [fixture.next_range(0, 6) as usize];
            let (word, bit) = initial.bit_position(key);
            initial.set_mask(terrain, word, bit);
            if fixture.next_range(0, 3) == 0 {
                initial.bb[BornSpace as usize][word] |= bit;
            }
            if fixture.next_range(0, 4) == 0 {
                initial.bb[BornLine as usize][word] |= bit;
            }
        }
        Ok(initial)
    }

    #[test]
    fn frontier_full_pipeline_preserves_complete_state() -> Result<(), MapSizeError> {
        for (width, height) in [
            (1, 1),
            (3, 2),
            (4, 3),
            (31, 7),
            (32, 1),
            (32, 2),
            (32, 33),
            (33, 32),
            (64, 5),
            (65, 3),
            (96, 96),
            (128, 128),
            (192, 192),
        ] {
            for seed in [
                0,
                1,
                -1,
                10_000,
                -12_345,
                17_547_620,
                321_251_673,
                614_279_151,
                755_583_854,
                1_235_228_408,
                1_235_545_220,
                1_333_158_113,
                i32::MAX,
                i32::MIN,
            ] {
                for stage in [ScoringStage::RawLing6, ScoringStage::Postprocessed] {
                    let mut reference = MapMaker::try_new(seed, width, height)?;
                    let mut candidate = MapMaker::try_new(seed, width, height)?;
                    candidate.set_kernel(GenerationKernel::Frontier);
                    assert_eq!(
                        reference.score(stage),
                        candidate.score(stage),
                        "{width}x{height}, seed={seed}, stage={stage:?}"
                    );
                    assert_eq!(
                        reference.diagnostic_state(),
                        candidate.diagnostic_state(),
                        "{width}x{height}, seed={seed}, stage={stage:?}"
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn frontier_passes_match_scalar_on_adversarial_states() -> Result<(), MapSizeError> {
        for (width, height) in [(32, 1), (32, 2), (32, 3), (64, 3), (96, 7)] {
            for seed in 0..16 {
                let initial = adversarial_state(seed, width, height)?;
                for condition in [
                    CType::AllTrue,
                    CType::NoBorn,
                    CType::CheckCon,
                    CType::CheckCon2,
                    CType::NoLing,
                    CType::OnlySoil,
                ] {
                    for terrain in [Soil, LingSoil, WetLand, RockBrown] {
                        for reverse in [false, true] {
                            for level in [-1, 0, 20, 99, 100] {
                                let rule = ExpansionRule {
                                    terrain,
                                    level,
                                    condition,
                                };
                                let mut reference = MapMaker::try_new(seed, width, height)?;
                                reference.bb.clone_from(&initial.bb);
                                let mut candidate = MapMaker::try_new(seed, width, height)?;
                                candidate.bb.clone_from(&initial.bb);
                                reference.expand_terrain_pass_scalar(rule, reverse);
                                assert!(candidate.expand_terrain_pass_frontier(rule, reverse));
                                assert_eq!(
                                    reference.diagnostic_state(),
                                    candidate.diagnostic_state(),
                                    "{width}x{height}, seed={seed}, terrain={terrain:?}, reverse={reverse}, level={level}"
                                );
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn frontier_handles_same_pass_chains_and_directed_origin_edges() -> Result<(), MapSizeError> {
        for reverse in [false, true] {
            for source in [0, 1, 31, 32, 33, 63, 64, 65, 127] {
                let mut reference = MapMaker::try_new(-1, 64, 2)?;
                reference.fill(Soil);
                let (word, bit) = reference.bit_position(source);
                reference.set_mask(LingSoil, word, bit);
                let mut candidate = MapMaker::try_new(-1, 64, 2)?;
                candidate.bb.clone_from(&reference.bb);
                let rule = ExpansionRule {
                    terrain: LingSoil,
                    level: 99,
                    condition: CType::CheckCon,
                };
                reference.expand_terrain_pass_scalar(rule, reverse);
                assert!(candidate.expand_terrain_pass_frontier(rule, reverse));
                assert_eq!(
                    reference.diagnostic_state(),
                    candidate.diagnostic_state(),
                    "source={source}, reverse={reverse}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn frontier_dimension_fallback_and_maximum_maps_preserve_state() -> Result<(), MapSizeError> {
        for (width, height) in [
            (1, 1),
            (3, 5),
            (4, 3),
            (31, 7),
            (33, 32),
            (36, 7),
            (65, 3),
            (32, 1024),
            (1024, 1),
            (1024, 3),
            (1024, 1024),
        ] {
            for reverse in [false, true] {
                let mut reference = MapMaker::try_new(1_235_545_220, width, height)?;
                reference.fill(Soil);
                for key in [
                    0,
                    1,
                    31,
                    32,
                    width - 1,
                    width,
                    width * height / 2,
                    width * height - 1,
                ] {
                    if (0..width * height).contains(&key) {
                        let (word, bit) = reference.bit_position(key);
                        reference.set_mask(LingSoil, word, bit);
                    }
                }
                let mut candidate = MapMaker::try_new(1_235_545_220, width, height)?;
                candidate.bb.clone_from(&reference.bb);
                candidate.set_kernel(GenerationKernel::Frontier);
                let rule = ExpansionRule {
                    terrain: LingSoil,
                    level: 99,
                    condition: CType::CheckCon,
                };
                if width % 32 != 0 {
                    let before = candidate.diagnostic_state();
                    let workspace = candidate.row4_eligible.clone();
                    assert!(!candidate.expand_terrain_pass_frontier(rule, reverse));
                    assert_eq!(candidate.diagnostic_state(), before);
                    assert_eq!(candidate.row4_eligible, workspace);
                }
                reference.expand_terrain_pass_scalar(rule, reverse);
                candidate.expand_terrain_pass(rule, reverse);
                assert_eq!(
                    reference.diagnostic_state(),
                    candidate.diagnostic_state(),
                    "{width}x{height}, reverse={reverse}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn frontier_index_preflight_and_fallback_preserve_state_in_both_directions(
    ) -> Result<(), MapSizeError> {
        for remaining in [0, 1, 63, 191, 192, 193] {
            for reverse in [false, true] {
                let mut reference = MapMaker::try_new(i32::MIN, 64, 3)?;
                reference.fill(Soil);
                reference.set_mask(LingSoil, 0, 0x8000_0003);
                reference
                    .rand
                    .advance_indexed(super::super::indexed_rng::MAX_DRAWS - remaining);
                let mut candidate = MapMaker::try_new(i32::MIN, 64, 3)?;
                candidate.bb.clone_from(&reference.bb);
                candidate.rand = reference.rand.clone();
                candidate.set_kernel(GenerationKernel::Frontier);
                let rule = ExpansionRule {
                    terrain: LingSoil,
                    level: 33,
                    condition: CType::CheckCon,
                };
                if remaining < 192 {
                    let before = candidate.diagnostic_state();
                    let workspace = candidate.row4_eligible.clone();
                    assert!(!candidate.expand_terrain_pass_frontier(rule, reverse));
                    assert_eq!(candidate.diagnostic_state(), before);
                    assert_eq!(candidate.row4_eligible, workspace);
                    candidate.expand_terrain_pass(rule, reverse);
                } else {
                    assert!(candidate.expand_terrain_pass_frontier(rule, reverse));
                }
                reference.expand_terrain_pass_scalar(rule, reverse);
                assert_eq!(
                    reference.diagnostic_state(),
                    candidate.diagnostic_state(),
                    "remaining={remaining}, reverse={reverse}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn frontier_make_map_exports_the_same_grid_and_complete_state() -> Result<(), MapSizeError> {
        for (width, height, seed) in [
            (32, 1, 0),
            (33, 3, -1),
            (64, 5, i32::MIN),
            (96, 96, 17_547_620),
            (192, 192, 321_251_673),
        ] {
            let mut reference = MapMaker::try_new(seed, width, height)?;
            let mut candidate = MapMaker::try_new(seed, width, height)?;
            candidate.set_kernel(GenerationKernel::Frontier);
            reference.make_map();
            candidate.make_map();
            assert_eq!(candidate.grid, reference.grid);
            assert_eq!(candidate.diagnostic_state(), reference.diagnostic_state());
        }
        Ok(())
    }
}

#[cfg(test)]
mod monotone_tests {
    use super::*;

    fn decoded(map: &MapMaker) -> MapStateSnapshot {
        let mut result = map.diagnostic_state();
        for word in 0..map.scan_base.len() {
            let absorbed = map.bb[Mud as usize][word] | map.bb[WetLand as usize][word];
            let fertile = map.bb[FertileSoil as usize][word];
            result.bitboards[FertileSoil as usize][word] = fertile & !absorbed;
            result.bitboards[LingSoil as usize][word] = map.scan_ling[word] & !(absorbed | fertile);
            result.bitboards[Soil as usize][word] =
                map.scan_base[word] & !(absorbed | fertile | map.scan_ling[word]);
        }
        result
    }

    #[test]
    fn quotient_matches_full_suffix_from_adversarial_partition_states() -> Result<(), MapSizeError>
    {
        for (width, height) in [
            (1, 1),
            (1, 7),
            (3, 3),
            (31, 33),
            (33, 31),
            (64, 7),
            (96, 96),
        ] {
            for seed in [0, -12_345, 321_251_673, i32::MIN] {
                for kernel in [
                    GenerationKernel::Scalar,
                    GenerationKernel::IndexedOutline,
                    GenerationKernel::Frontier,
                ] {
                    let mut reference = MapMaker::try_new(seed, width, height)?;
                    let mut candidate = MapMaker::try_new(seed, width, height)?;
                    for map in [&mut reference, &mut candidate] {
                        map.set_kernel(kernel);
                        map.fill(Soil);
                        for key in 0..width * height {
                            let slot = ((key as u32).wrapping_mul(0x9e37_79b9)
                                ^ (seed as u32).rotate_left(13))
                                % 11;
                            let terrain = [
                                Soil,
                                FertileSoil,
                                Mud,
                                LingSoil,
                                WetLand,
                                StoneLand,
                                Soil,
                                Soil,
                                Soil,
                                LingSoil,
                                RockGray,
                            ][slot as usize];
                            map.apply_allowed(terrain, key, CType::AllTrue);
                        }
                    }
                    reference.postprocess_ling_overwrites();
                    candidate.postprocess_ling_overwrites_impl(true);
                    assert_eq!(
                        decoded(&candidate),
                        reference.diagnostic_state(),
                        "{width}x{height}, seed={seed}, {kernel:?}"
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod elision_tests {
    use super::*;

    #[test]
    fn born_outline_elision_matches_original_on_all_three_by_three_fertile_masks(
    ) -> Result<(), MapSizeError> {
        for (width, height) in [(1, 1), (2, 1), (1, 3), (3, 3)] {
            let cells = width * height;
            for bits in 0u32..(1u32 << cells) {
                let mut reference = MapMaker::try_new(-12_345, width, height)?;
                let mut candidate = MapMaker::try_new(-12_345, width, height)?;
                for map in [&mut reference, &mut candidate] {
                    map.set_kernel(GenerationKernel::Scalar);
                    map.fill(Soil);
                    for key in 0..cells {
                        if bits & (1u32 << key) != 0 {
                            map.apply_allowed(FertileSoil, key, CType::AllTrue);
                        }
                    }
                    map.snapshot_born_space();
                }
                reference.out_line(FertileSoil, FertileSoil, 2, 100, 0, CType::AllTrue);
                reference.clear_born_fertile_soil();
                candidate.elide_born_fertile_outline_and_clear();
                assert_eq!(
                    reference.diagnostic_state(),
                    candidate.diagnostic_state(),
                    "{width}x{height}: initial fertile mask={bits}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn zero_radius_self_outline_matches_unelided_visit_loop() -> Result<(), MapSizeError> {
        for terrain in [
            Soil,
            FertileSoil,
            IronOre,
            CopperOre,
            SilverOre,
            RockGray,
            RockMarble,
        ] {
            for condition in [
                CType::AllTrue,
                CType::NoBorn,
                CType::CheckCon,
                CType::NoLing,
            ] {
                for (width, height) in [(1, 1), (3, 2), (33, 5), (96, 96)] {
                    let mut reference = MapMaker::try_new(i32::MIN, width, height)?;
                    let mut candidate = MapMaker::try_new(i32::MIN, width, height)?;
                    for map in [&mut reference, &mut candidate] {
                        map.fill(Soil);
                        for key in 0..width * height {
                            if key % 3 == 0 {
                                map.apply_allowed(terrain, key, CType::AllTrue);
                            }
                        }
                    }
                    // Explicitly bypass the new dispatch to retain a sequential oracle.
                    reference.out_line_batched(terrain, terrain, 0, 4, condition);
                    candidate.out_line(terrain, terrain, 0, 4, 0, condition);
                    assert_eq!(
                        reference.diagnostic_state(),
                        candidate.diagnostic_state(),
                        "{width}x{height}, {terrain:?}, {condition:?}"
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod word_research_tests {
    use super::*;
    fn reference(active: u32, eligible: u32, random: u32, n: u32) -> (u32, u32) {
        let mut sources = 0;
        let mut consumed = 0;
        for x in 0..n {
            let on = active & (1 << x) != 0
                || (x > 0 && eligible & (1 << x) != 0 && sources & (1 << (x - 1)) != 0);
            if on {
                if random & (1 << consumed) != 0 {
                    sources |= 1 << x;
                }
                consumed += 1;
            }
        }
        (sources, consumed)
    }
    #[test]
    fn exhaustive_interval_transducer_length_7() {
        // E may overlap A after earlier rows paint originally eligible cells.
        let limit = std::hint::black_box(128u32);
        for a in 0..limit {
            for e in 0..limit {
                for r in 0..limit {
                    assert_eq!(
                        word_run_transducer(a, e, r),
                        reference(a, e, r, 7),
                        "A={a},E={e},R={r}"
                    );
                }
            }
        }
        for (a, e, r) in [
            (0, !0, !0),
            (!0, !0, !0),
            (!0, 0, 0),
            (1, !0, !0),
            (1, !0, 0),
            (1 << 31, !0, !0),
            (0x5555_5555, !0, 0xaaaa_aaaa),
            (0xaaaa_aaaa, !0, 0x5555_5555),
        ] {
            assert_eq!(word_run_transducer(a, e, r), reference(a, e, r, 32));
        }
    }
    #[test]
    fn whole_word_pass_matches_scalar_random_states() -> Result<(), MapSizeError> {
        let mut gen = 17u64;
        for &(width, height) in &[
            (1, 1),
            (1, 3),
            (2, 7),
            (7, 3),
            (31, 3),
            (32, 3),
            (33, 3),
            (63, 5),
            (64, 5),
            (65, 3),
            (96, 3),
        ] {
            for seed in [0, 1, -1, 10000, i32::MIN, i32::MAX] {
                for reverse in [false, true] {
                    for level in [-1, 0, 20, 33, 98, 99, 100] {
                        let (width, height, seed, reverse, level) =
                            std::hint::black_box((width, height, seed, reverse, level));
                        let mut a = MapMaker::try_new(seed, width, height)?;
                        a.fill(Soil);
                        for word in 0..a.bb[LingSoil as usize].len() {
                            gen = gen.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                            let valid = a.get_ctype_mask(word, CType::OnlySoil);
                            let mask = gen as u32 & valid;
                            a.set_mask(LingSoil, word, mask);
                            a.set_mask(RockBrown, word, (gen >> 32) as u32 & valid & !mask);
                        }
                        let mut b = MapMaker::try_new(seed, width, height)?;
                        b.bb = a.bb.clone();
                        b.rand = a.rand.clone();
                        let rule = ExpansionRule {
                            terrain: LingSoil,
                            level,
                            condition: CType::CheckCon,
                        };
                        a.expand_terrain_pass_scalar(rule, reverse);
                        b.expand_terrain_pass_word(rule, reverse);
                        assert_eq!(
                            a.diagnostic_state(),
                            b.diagnostic_state(),
                            "{width}x{height} s={seed} rev={reverse} level={level}"
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod early_quotient_tests {
    use super::*;
    #[test]
    fn early_quotient_restores_complete_water_state() -> Result<(), MapSizeError> {
        for (w, h) in [
            (1, 1),
            (1, 7),
            (7, 1),
            (3, 5),
            (31, 33),
            (33, 31),
            (32, 32),
            (64, 3),
            (96, 96),
            (128, 128),
            (192, 192),
            (1024, 1),
            (1, 1024),
        ] {
            for seed in [0, 1, -1, 10000, 29999, i32::MIN, i32::MAX] {
                for kernel in [
                    GenerationKernel::Scalar,
                    GenerationKernel::IndexedOutline,
                    GenerationKernel::Frontier,
                ] {
                    let mut a = MapMaker::try_new(seed, w, h)?;
                    let mut b = MapMaker::try_new(seed, w, h)?;
                    a.set_kernel(kernel);
                    b.set_kernel(kernel);
                    b.scan_early_ground = true;
                    for m in [&mut a, &mut b] {
                        m.build_base_topology();
                        m.optimize(FertileSoil, 2, 4, 1, CType::AllTrue);
                        m.build_water();
                    }
                    assert_eq!(
                        a.diagnostic_state(),
                        b.diagnostic_state(),
                        "{w}x{h} s={seed} kernel={kernel:?}"
                    );
                }
            }
        }
        Ok(())
    }
}
