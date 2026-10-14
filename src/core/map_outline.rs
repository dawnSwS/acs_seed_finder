//! Exact target-wise evaluation of source-snapshot `OutLine` operations.
//!
//! Every original valid visit keeps its original RNG index. A target is written
//! iff any eligible visit passes, so repeated writes to that same terrain are
//! evaluated as an OR. Skipped queries still advance the logical RNG stream.
use super::{CType, MapMaker, Terrain, Tmp1};
use crate::core::constants::get_base_around_50;
use std::cmp::Ordering;
use std::sync::OnceLock;

struct RadiusGeometry {
    radius: i32,
    side: usize,
    rank: Vec<u8>,
    category: Vec<u16>,
}

impl RadiusGeometry {
    fn new(map_size: i32, radius: i32) -> Self {
        let side = (radius * 2 + 1) as usize;
        let mut rank = vec![u8::MAX; side.pow(4)];
        let mut category = vec![0; (map_size * map_size) as usize];
        let class = |coordinate: i32| {
            if coordinate < radius {
                coordinate
            } else if coordinate >= map_size - radius {
                2 * radius - (map_size - 1 - coordinate)
            } else {
                radius
            }
        };
        let representative = |class: i32| match class.cmp(&radius) {
            Ordering::Less => class,
            Ordering::Equal => map_size / 2,
            Ordering::Greater => map_size - 1 - (2 * radius - class),
        };
        for y in 0..map_size {
            for x in 0..map_size {
                category[(y * map_size + x) as usize] = (class(y) * side as i32 + class(x)) as u16;
            }
        }
        for cy in 0..side {
            for cx in 0..side {
                let (x, y) = (representative(cx as i32), representative(cy as i32));
                let mut valid_rank = 0;
                for &(dx, dy) in get_base_around_50().iter().take(side * side) {
                    if !(0..map_size).contains(&(x + dx)) || !(0..map_size).contains(&(y + dy)) {
                        continue;
                    }
                    let index = ((cy * side + cx) * side + (dy + radius) as usize) * side
                        + (dx + radius) as usize;
                    rank[index] = valid_rank;
                    valid_rank += 1;
                }
            }
        }
        Self {
            radius,
            side,
            rank,
            category,
        }
    }

    #[inline]
    fn rank(&self, source: usize, dx: i32, dy: i32) -> u64 {
        let index = (usize::from(self.category[source]) * self.side + (dy + self.radius) as usize)
            * self.side
            + (dx + self.radius) as usize;
        let rank = self.rank[index];
        debug_assert_ne!(rank, u8::MAX);
        u64::from(rank)
    }
}

fn geometry(size: i32) -> &'static [RadiusGeometry; 6] {
    static G96: OnceLock<[RadiusGeometry; 6]> = OnceLock::new();
    static G128: OnceLock<[RadiusGeometry; 6]> = OnceLock::new();
    static G192: OnceLock<[RadiusGeometry; 6]> = OnceLock::new();
    let cell = match size {
        96 => &G96,
        128 => &G128,
        192 => &G192,
        _ => unreachable!(),
    };
    cell.get_or_init(|| std::array::from_fn(|radius| RadiusGeometry::new(size, radius as i32)))
}

pub(super) fn prepare_geometry() {
    for size in [96, 128, 192] {
        let _ = geometry(size);
    }
}

pub(super) struct OutlineWorkspace {
    source_start: Vec<u64>,
    source_kind: Vec<u8>,
    source: Vec<u32>,
    horizontal: Vec<u32>,
    coverage: Vec<u32>,
}

impl OutlineWorkspace {
    fn new(size: usize) -> Self {
        let words = size / 32 * size;
        Self {
            source_start: vec![0; size * size],
            source_kind: vec![0; size * size],
            source: vec![0; words],
            horizontal: vec![0; words],
            coverage: vec![0; words],
        }
    }

    /// Constant-threshold calls never query a source start. Sum each whole
    /// word's visit weights instead of enumerating and indexing its sources.
    fn total_visits(&self, size: usize, radius: usize) -> u64 {
        let stride = size / 32;
        let diameter = (2 * radius + 1) as u64;
        let mut total = 0;
        for row in 0..size {
            let height = ((row + radius).min(size - 1) - row.saturating_sub(radius) + 1) as u64;
            for column in 0..stride {
                let bits = self.source[row * stride + column];
                let mut width_sum = u64::from(bits.count_ones()) * diameter;
                if column == 0 {
                    for distance in 0..radius {
                        width_sum -= u64::from((bits >> distance) & 1) * (radius - distance) as u64;
                    }
                } else if column + 1 == stride {
                    for distance in 0..radius {
                        width_sum -=
                            u64::from((bits >> (31 - distance)) & 1) * (radius - distance) as u64;
                    }
                }
                total += height * width_sum;
            }
        }
        total
    }

    /// Separable bitboard dilation: O(words * radius), not a per-cell square scan.
    fn cover(&mut self, size: usize, radius: usize) {
        let stride = size / 32;
        for word in 0..self.source.len() {
            let column = word % stride;
            let center = self.source[word];
            let left = if column > 0 { self.source[word - 1] } else { 0 };
            let right = if column + 1 < stride {
                self.source[word + 1]
            } else {
                0
            };
            let mut mask = center;
            for distance in 1..=radius {
                mask |= (center << distance)
                    | (center >> distance)
                    | (left >> (32 - distance))
                    | (right << (32 - distance));
            }
            self.horizontal[word] = mask;
        }
        for row in 0..size {
            for column in 0..stride {
                let mut mask = 0;
                for source_row in row.saturating_sub(radius)..=(row + radius).min(size - 1) {
                    mask |= self.horizontal[source_row * stride + column];
                }
                self.coverage[row * stride + column] = mask;
            }
        }
    }

    /// Visit covering sources in the original descending row/bit order.
    #[inline]
    fn any_source_passes(
        &self,
        size: i32,
        radius: i32,
        target: (i32, i32),
        mut accepts: impl FnMut(usize, i32, i32) -> bool,
    ) -> bool {
        let stride = size as usize / 32;
        let (x, y) = target;
        let (lx, hx, ly, hy) = (
            (x - radius).max(0),
            (x + radius).min(size - 1),
            (y - radius).max(0),
            (y + radius).min(size - 1),
        );
        for sy in (ly..=hy).rev() {
            for source_word in (lx as usize / 32..=hx as usize / 32).rev() {
                let first = (lx - source_word as i32 * 32).max(0) as u32;
                let last = (hx - source_word as i32 * 32).min(31) as u32;
                let mut sources = self.source[sy as usize * stride + source_word]
                    & (u32::MAX << first)
                    & (u32::MAX >> (31 - last));
                while sources != 0 {
                    let bit = sources.ilog2();
                    sources ^= 1 << bit;
                    let sx = (source_word * 32 + bit as usize) as i32;
                    let key = (sy * size + sx) as usize;
                    if accepts(key, x - sx, y - sy) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[derive(Clone, Copy, Default)]
struct Stream {
    start: u64,
    bound: i32,
}

impl MapMaker {
    pub(super) fn supports_indexed_outlines(&self) -> bool {
        self.w == self.h && matches!(self.w, 96 | 128 | 192)
    }

    /// Returns false without any terrain/RNG mutation when the candidate
    /// cannot cover this call. The caller then executes its sequential path.
    pub(super) fn out_line_indexed(
        &mut self,
        src: Terrain,
        tgt: Terrain,
        radius: i32,
        level: i32,
        condition: CType,
    ) -> bool {
        if !self.supports_indexed_outlines()
            || !(0..=5).contains(&radius)
            || self.rand.draw_position() >= crate::core::indexed_rng::MAX_DRAWS as u64
        {
            return false;
        }
        let size = self.w as usize;
        let stride = size / 32;
        let mut workspace = self
            .outline_workspace
            .take()
            .unwrap_or_else(|| OutlineWorkspace::new(size));
        workspace.source.copy_from_slice(&self.bb[src as usize]);
        workspace.source[0] &= !1; // source key zero is excluded; target zero is legal.
        let initial = self.rand.draw_position();
        let mut offset = initial;
        if level >= 99 {
            offset += workspace.total_visits(size, radius as usize);
        } else {
            for word in (0..workspace.source.len()).rev() {
                let mut bits = workspace.source[word];
                while bits != 0 {
                    let bit = bits.ilog2();
                    bits ^= 1 << bit;
                    let (x, y) = (
                        (word % stride * 32 + bit as usize) as i32,
                        (word / stride) as i32,
                    );
                    workspace.source_start[(y * self.w + x) as usize] = offset;
                    offset += ((x + radius).min(self.w - 1) - (x - radius).max(0) + 1) as u64
                        * ((y + radius).min(self.h - 1) - (y - radius).max(0) + 1) as u64;
                }
            }
        }
        let count = (offset - initial) as usize;
        let Some(reader) = self.rand.indexed_reader(count) else {
            self.outline_workspace = Some(workspace);
            return false;
        };
        let bound = crate::core::rng::threshold_bound(level);
        let (layers, temporary) = self.bb.split_at_mut(Tmp1 as usize);
        temporary[0].copy_from_slice(&layers[src as usize]);
        let geometry = &geometry(self.w)[radius as usize];
        workspace.cover(size, radius as usize);
        for word in 0..workspace.coverage.len() {
            let mut todo = workspace.coverage[word]
                & self.get_ctype_mask(word, condition)
                & !self.bb[tgt as usize][word];
            let mut accepted = if level >= 99 { todo } else { 0 };
            if (0..99).contains(&level) {
                while todo != 0 {
                    let bit = todo.trailing_zeros();
                    todo &= todo - 1;
                    let (x, y) = (
                        (word % stride * 32 + bit as usize) as i32,
                        (word / stride) as i32,
                    );
                    let hit = workspace.any_source_passes(self.w, radius, (x, y), |key, dx, dy| {
                        let index = workspace.source_start[key] + geometry.rank(key, dx, dy);
                        reader.raw_at(index) <= bound
                    });
                    if hit {
                        accepted |= 1 << bit;
                    }
                }
            }
            if accepted != 0 {
                self.set_allowed_mask(tgt, word, accepted, condition);
            }
        }
        self.rand.advance_indexed(count);
        self.outline_workspace = Some(workspace);
        true
    }

    #[inline]
    fn index_group_source(
        &self,
        workspace: &mut OutlineWorkspace,
        src: Terrain,
        condition: CType,
    ) -> u64 {
        let stride = self.w as usize / 32;
        let mut local = 0;
        for word in (0..workspace.source.len()).rev() {
            let mut bits = super::without_origin(word, self.bb[src as usize][word]);
            debug_assert_eq!(
                bits & self.get_ctype_mask(word, condition),
                0,
                "a fused source must not be writable"
            );
            workspace.source[word] |= bits;
            while bits != 0 {
                let bit = bits.ilog2();
                bits ^= 1 << bit;
                let (x, y) = (
                    (word % stride * 32 + bit as usize) as i32,
                    (word / stride) as i32,
                );
                let key = (y * self.w + x) as usize;
                workspace.source_kind[key] = src as u8;
                workspace.source_start[key] = local;
                local += ((x + 1).min(self.w - 1) - (x - 1).max(0) + 1) as u64
                    * ((y + 1).min(self.h - 1) - (y - 1).max(0) + 1) as u64;
            }
        }
        local
    }

    /// Fuses only a proven source-stable group of radius-one calls sharing a
    /// destination and predicate. Repeated source terrains retain distinct RNG
    /// segments. No draw is removed, including the singleton ranges between
    /// the three low-level ore-border calls (which consume zero draws).
    pub(super) fn out_line_group_indexed(
        &mut self,
        specs: &[(Terrain, i32)],
        tgt: Terrain,
        condition: CType,
    ) -> bool {
        if !self.supports_indexed_outlines()
            || specs.is_empty()
            || self.rand.draw_position() >= crate::core::indexed_rng::MAX_DRAWS as u64
        {
            return false;
        }
        let size = self.w as usize;
        let stride = size / 32;
        let mut workspace = self
            .outline_workspace
            .take()
            .unwrap_or_else(|| OutlineWorkspace::new(size));
        workspace.source.fill(0);
        let mut streams = [[Stream::default(); 2]; 20];
        let mut stream_counts = [0usize; 20];
        let initial = self.rand.draw_position();
        let mut offset = initial;
        for &(src, level) in specs {
            debug_assert_ne!(src, tgt);
            let local = self.index_group_source(&mut workspace, src, condition);
            let count = &mut stream_counts[src as usize];
            assert!(
                *count < 2,
                "fusion supports at most two calls per source terrain"
            );
            streams[src as usize][*count] = Stream {
                start: offset,
                bound: crate::core::rng::threshold_bound(level),
            };
            *count += 1;
            offset += local;
        }
        let count = (offset - initial) as usize;
        let Some(reader) = self.rand.indexed_reader(count) else {
            self.outline_workspace = Some(workspace);
            return false;
        };
        // Sources are fixed throughout the group, so this is also the final
        // sequential call's exact source scratch snapshot.
        let last_src = specs[specs.len() - 1].0;
        let (layers, temporary) = self.bb.split_at_mut(Tmp1 as usize);
        temporary[0].copy_from_slice(&layers[last_src as usize]);
        let geometry = &geometry(self.w)[1];
        workspace.cover(size, 1);
        for word in 0..workspace.coverage.len() {
            let mut todo = workspace.coverage[word]
                & self.get_ctype_mask(word, condition)
                & !self.bb[tgt as usize][word];
            let mut accepted = 0;
            while todo != 0 {
                let bit = todo.trailing_zeros();
                todo &= todo - 1;
                let (x, y) = (
                    (word % stride * 32 + bit as usize) as i32,
                    (word / stride) as i32,
                );
                let hit = workspace.any_source_passes(self.w, 1, (x, y), |key, dx, dy| {
                    let kind = usize::from(workspace.source_kind[key]);
                    let relative = workspace.source_start[key] + geometry.rank(key, dx, dy);
                    streams[kind][..stream_counts[kind]]
                        .iter()
                        .any(|stream| reader.raw_at(stream.start + relative) <= stream.bound)
                });
                if hit {
                    accepted |= 1 << bit;
                }
            }
            if accepted != 0 {
                self.set_allowed_mask(tgt, word, accepted, condition);
            }
        }
        self.rand.advance_indexed(count);
        self.outline_workspace = Some(workspace);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::{FertileSoil, GenerationKernel, MapSizeError, ScoringStage, Soil};
    use super::*;

    #[test]
    fn constant_threshold_total_matches_direct_source_enumeration() {
        for size in [96usize, 128, 192] {
            let stride = size / 32;
            let mut workspace = OutlineWorkspace::new(size);
            for radius in 0..=5 {
                for pattern in 0..4 {
                    for (word, bits) in workspace.source.iter_mut().enumerate() {
                        *bits = match pattern {
                            0 => u32::MAX,
                            1 => 0x8000_0001,
                            2 => 1u32 << (word % 32),
                            _ => (word as u32)
                                .wrapping_mul(1_664_525)
                                .wrapping_add(1_013_904_223),
                        };
                    }
                    workspace.source[0] &= !1;
                    let mut expected = 0;
                    for (word, &bitmap) in workspace.source.iter().enumerate() {
                        let mut bits = bitmap;
                        while bits != 0 {
                            let bit = bits.trailing_zeros() as usize;
                            bits &= bits - 1;
                            let x = word % stride * 32 + bit;
                            let y = word / stride;
                            expected += ((x + radius).min(size - 1) - x.saturating_sub(radius) + 1)
                                as u64
                                * ((y + radius).min(size - 1) - y.saturating_sub(radius) + 1)
                                    as u64;
                        }
                    }
                    assert_eq!(workspace.total_visits(size, radius), expected);
                }
            }
        }
    }

    #[test]
    fn constant_threshold_keeps_full_state_at_clipped_boundaries() -> Result<(), MapSizeError> {
        for size in [96, 128, 192] {
            for radius in 0..=5 {
                for level in [99, 100] {
                    let mut reference = MapMaker::try_new(17_547_620, size, size)?;
                    reference.set_kernel(GenerationKernel::Scalar);
                    let mut candidate = MapMaker::try_new(17_547_620, size, size)?;
                    candidate.set_kernel(GenerationKernel::IndexedOutline);
                    for map in [&mut reference, &mut candidate] {
                        map.fill(Soil);
                        for y in [0, 1, 4, size / 2, size - 5, size - 1] {
                            for x in [0, 1, 4, 31, 32, size / 2, size - 5, size - 1] {
                                map.apply_allowed(FertileSoil, y * size + x, CType::AllTrue);
                            }
                        }
                        map.out_line(
                            FertileSoil,
                            super::super::WetLand,
                            radius,
                            level,
                            0,
                            CType::AllTrue,
                        );
                    }
                    assert_eq!(
                        reference.diagnostic_state(),
                        candidate.diagnostic_state(),
                        "size={size}, radius={radius}, level={level}"
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn clipped_geometry_retains_exact_valid_spiral_ranks() {
        for size in [96, 128, 192] {
            let coordinates = [
                0,
                1,
                2,
                4,
                5,
                size / 2,
                size - 6,
                size - 5,
                size - 2,
                size - 1,
            ];
            for radius in 0..=5 {
                let geometry = &geometry(size)[radius as usize];
                for x in coordinates {
                    for y in coordinates {
                        let mut rank = 0;
                        for &(dx, dy) in get_base_around_50()
                            .iter()
                            .take(((radius * 2 + 1) * (radius * 2 + 1)) as usize)
                        {
                            if !(0..size).contains(&(x + dx)) || !(0..size).contains(&(y + dy)) {
                                continue;
                            }
                            assert_eq!(geometry.rank((y * size + x) as usize, dx, dy), rank);
                            rank += 1;
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn indexed_outline_preserves_full_maps_and_rng_at_both_stages() -> Result<(), MapSizeError> {
        for (width, height, seed) in [
            (96, 96, 1),
            (96, 96, i32::MIN),
            (128, 128, 17_547_620),
            (128, 128, -12_345),
            (192, 192, 321_251_673),
            (192, 192, 2_147_483_647),
            (33, 31, 321_251_673),
        ] {
            for stage in [ScoringStage::RawLing6, ScoringStage::Postprocessed] {
                let mut reference = MapMaker::try_new(seed, width, height)?;
                reference.set_kernel(GenerationKernel::Scalar);
                let mut candidate = MapMaker::try_new(seed, width, height)?;
                candidate.set_kernel(GenerationKernel::IndexedOutline);
                assert_eq!(
                    reference.score(stage),
                    candidate.score(stage),
                    "{width}x{height}, seed={seed}, {stage:?}"
                );
                assert_eq!(
                    reference.diagnostic_state(),
                    candidate.diagnostic_state(),
                    "{width}x{height}, seed={seed}, {stage:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn table_end_and_nonzero_maximum_use_exact_sequential_fallback() -> Result<(), MapSizeError> {
        for maximum in [0, 3] {
            let mut reference = MapMaker::try_new(321_251_673, 96, 96)?;
            reference.set_kernel(GenerationKernel::Scalar);
            let mut candidate = MapMaker::try_new(321_251_673, 96, 96)?;
            candidate.set_kernel(GenerationKernel::IndexedOutline);
            for map in [&mut reference, &mut candidate] {
                map.fill(Soil);
                map.apply_allowed(FertileSoil, 97, CType::AllTrue);
                map.rand.advance(1_999_995);
                map.out_line(FertileSoil, FertileSoil, 2, 33, maximum, CType::AllTrue);
            }
            assert_eq!(reference.diagnostic_state(), candidate.diagnostic_state());
        }
        Ok(())
    }
}
