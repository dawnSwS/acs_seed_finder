// Arithmetic, narrowing conversions and fixed-array indexing intentionally
// reproduce the supplied game's unchecked C# RNG, not host-native randomness.
#![allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::indexing_slicing
)]

#[derive(Clone)]
pub struct GRandom {
    pub seed: u32,
}
impl GRandom {
    pub fn new(seed: u32) -> Self {
        Self { seed }
    }
    pub fn rand(&mut self) -> u32 {
        self.seed = u64::from(self.seed)
            .wrapping_mul(1_103_515_245)
            .wrapping_add(12345) as u32;
        self.seed = self.seed.rotate_left(16);
        self.seed
    }
    pub fn rand_range(&mut self, n_min: i32, n_max: i32) -> i32 {
        // C# semantics: GRandom.RandRange(nMin, nMax) with closed interval.
        // Rust call convention: rand_range(a, b) → C# RandRange(a, b-1).
        let mut c_min = n_min;
        let mut c_max = n_max.wrapping_sub(1);

        // C# swaps if nMin > nMax
        if c_min > c_max {
            std::mem::swap(&mut c_min, &mut c_max);
        }

        // Always consume one Rand() to advance state
        let ratio = f64::from(self.rand()) / 4_294_967_295.0;
        let span = c_max.wrapping_sub(c_min).wrapping_add(1);
        let num2 = (ratio * (f64::from(span))) as i32;
        let mut num3 = num2.wrapping_add(c_min);

        // C# clamps: if (num3 > nMax) num3 = nMax
        if num3 > c_max {
            num3 = c_max;
        }

        num3
    }
    pub fn rand_float(&mut self, mut min: f32, mut max: f32) -> f32 {
        // C# semantics: GRandom.RandRange(float fMin, float fMax)
        // Always swap if min > max, always consume one Rand()
        if min > max {
            std::mem::swap(&mut min, &mut max);
        }

        let ratio = f64::from(self.rand()) / 4_294_967_295.0;
        // Single f64 multiplication, then truncate to f32 (matching C# exactly)
        let num2 = (ratio * f64::from(max - min)) as f32;

        num2 + min
    }
}

/// The legacy Mono `System.Random` shipped with the supplied game.
/// Its lag is 31, unlike Microsoft's implementation (21), and integer
/// ranges of length zero or one do not advance the state.
#[derive(Clone, Debug)]
pub struct DotNetRandom {
    inext: usize,
    inextp: usize,
    seed_array: [i32; 56],
    indexed_seed: super::indexed_rng::Seed,
    draw_position: u64,
    // If true, the ring is an old checkpoint. (indexed_seed, draw_position)
    // is the complete logical state; materialize only before sequential use.
    ring_dirty: bool,
}

impl PartialEq for DotNetRandom {
    fn eq(&self, other: &Self) -> bool {
        self.indexed_seed == other.indexed_seed
            && self.draw_position == other.draw_position
            && self.diagnostic_state() == other.diagnostic_state()
    }
}
impl Eq for DotNetRandom {}

// A vector's pointer must carry its alignment even when the ring itself is
// unaligned. Copies into and out of these lanes keep that requirement explicit.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[repr(C, align(32))]
struct Avx2Lanes([i32; 8]);

impl DotNetRandom {
    pub fn new(seed: i32) -> Self {
        let mut r = Self {
            inext: 0,
            inextp: 31,
            seed_array: [0; 56],
            indexed_seed: super::indexed_rng::Seed::new(seed),
            draw_position: 0,
            ring_dirty: false,
        };
        // Preserve this scanner's existing i32::MIN convention. The supplied
        // Mono Math.Abs would throw there; changing that API is outside batching.
        let mut mj = 161_803_398 - seed.saturating_abs();
        r.seed_array[55] = mj;
        let mut mk: i32 = 1;
        for i in 1..55 {
            let ii = (21 * i) % 55;
            r.seed_array[ii] = mk;
            // The game's unchecked C# subtraction wraps for extreme seeds.
            mk = mj.wrapping_sub(mk);
            if mk < 0 {
                mk += i32::MAX;
            }
            mj = r.seed_array[ii];
        }
        for _ in 1..5 {
            for k in 1..56 {
                let mut val = r.seed_array[k].wrapping_sub(r.seed_array[1 + (k + 30) % 55]);
                if val < 0 {
                    val += i32::MAX;
                }
                r.seed_array[k] = val;
            }
        }
        r
    }
    #[inline]
    fn next_raw(&mut self) -> i32 {
        if self.ring_dirty {
            if let Some(value) = super::indexed_rng::raw_at(self.indexed_seed, self.draw_position) {
                self.draw_position += 1;
                return value;
            }
        }
        self.materialize_ring();
        // Saturation disables indexing rather than changing the RNG stream if
        // an impractically long run ever exhausts the diagnostic counter.
        self.draw_position = self.draw_position.saturating_add(1);
        self.inext = if self.inext + 1 == 56 {
            1
        } else {
            self.inext + 1
        };
        self.inextp = if self.inextp + 1 == 56 {
            1
        } else {
            self.inextp + 1
        };
        let mut n = self.seed_array[self.inext].wrapping_sub(self.seed_array[self.inextp]);
        if n < 0 {
            n += i32::MAX;
        }
        self.seed_array[self.inext] = n;
        n
    }
    pub fn next_double(&mut self) -> f64 {
        f64::from(self.next_raw()) * 4.656_612_875_245_797e-10
    }

    /// The complete state, for differential checks against the original generator.
    pub fn diagnostic_state(&self) -> ([i32; 56], usize, usize) {
        if self.ring_dirty {
            let Some(state) =
                super::indexed_rng::state_after(self.indexed_seed, self.draw_position)
            else {
                unreachable!("lazy ring invariant: indexed state remains available");
            };
            return state;
        }
        (self.seed_array, self.inext, self.inextp)
    }

    #[inline]
    fn materialize_ring(&mut self) {
        if self.ring_dirty {
            let (slots, p, q) = self.diagnostic_state();
            self.seed_array = slots;
            self.inext = p;
            self.inextp = q;
            self.ring_dirty = false;
        }
    }

    /// Zero-based position of the next sample; independent of range widths.
    pub fn draw_position(&self) -> u64 {
        self.draw_position
    }

    /// Preflight a whole indexed operation before changing any map state.
    /// This also initializes the shared coefficient table on its first use.
    pub fn indexed_available(&self, count: usize) -> bool {
        self.draw_position
            .checked_add(count as u64)
            .is_some_and(super::indexed_rng::available)
    }

    /// Preflight once and capture the immutable table for a whole operation.
    /// Its indices are absolute, just like `indexed_threshold_at`.
    pub(super) fn indexed_reader(&self, count: usize) -> Option<super::indexed_rng::Reader> {
        let end = self.draw_position.checked_add(count as u64)?;
        super::indexed_rng::reader(self.indexed_seed, end)
    }

    /// Exact `Next(0,100) <= level` at an absolute zero-based sample index.
    /// None means the caller must use its sequential fallback. No RNG state is
    /// changed, including when the requested index is outside the table.
    pub fn indexed_threshold_at(&self, zero_based: u64, level: i32) -> Option<bool> {
        super::indexed_rng::raw_at(self.indexed_seed, zero_based)
            .map(|raw| raw <= threshold_bound(level))
    }

    /// Skip exactly `count` samples using indexed state reconstruction when
    /// possible; short skips and targets outside the table remain sequential.
    pub fn advance_indexed(&mut self, count: usize) {
        if count == 0 {
            return;
        }
        if count >= 128 {
            if let Some(target) = self.draw_position.checked_add(count as u64) {
                if target >= 55 && super::indexed_rng::available(target) {
                    self.draw_position = target;
                    self.ring_dirty = true;
                    return;
                }
            }
        }
        self.advance(count);
    }

    /// Backend selected by `next_threshold_mask_32` on this CPU.
    pub fn batch_backend() -> &'static str {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        if std::is_x86_feature_detected!("avx2") {
            return "avx2";
        }
        "scalar"
    }

    /// Observe the next 32 `Next(0, 100) <= level` results without consuming them.
    /// Bit zero corresponds to the first sample.
    pub fn peek_threshold_mask_32(&self, level: i32) -> u32 {
        self.clone().next_threshold_mask_32(level)
    }

    /// Consume exactly 32 samples and return `Next(0, 100) <= level` as bits.
    /// AVX2 is selected at runtime; other CPUs use the identical scalar stream.
    /// Even levels outside 0..=98 consume all 32 samples, just like Next calls.
    pub fn next_threshold_mask_32(&mut self, level: i32) -> u32 {
        let bound = threshold_bound(level);
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        if std::is_x86_feature_detected!("avx2") {
            // SAFETY: the target feature is checked before entering this function.
            return unsafe { self.threshold_mask_32_avx2(bound) };
        }
        self.threshold_mask_32_scalar(bound)
    }

    /// Force the portable batch kernel for reproducible backend comparisons.
    /// This has exactly the same consumption and bit order as the dispatched API.
    pub fn next_threshold_mask_32_scalar(&mut self, level: i32) -> u32 {
        self.threshold_mask_32_scalar(threshold_bound(level))
    }

    /// Discard exactly `count` samples. Short checkpoint tails stay scalar.
    pub fn advance(&mut self, mut count: usize) {
        while count >= 32 {
            self.next_threshold_mask_32(99);
            count -= 32;
        }
        for _ in 0..count {
            self.next_raw();
        }
    }

    #[inline]
    fn threshold_mask_32_scalar(&mut self, bound: i32) -> u32 {
        self.materialize_ring();
        let mut bits = 0;
        for shift in 0..32 {
            bits |= u32::from(self.next_raw() <= bound) << shift;
        }
        bits
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    #[target_feature(enable = "avx2")]
    unsafe fn threshold_mask_32_avx2(&mut self, bound: i32) -> u32 {
        #[cfg(target_arch = "x86")]
        use std::arch::x86::{
            __m256i, _mm256_add_epi32, _mm256_and_si256, _mm256_castsi256_ps, _mm256_cmpgt_epi32,
            _mm256_load_si256, _mm256_movemask_ps, _mm256_set1_epi32, _mm256_srai_epi32,
            _mm256_store_si256, _mm256_sub_epi32,
        };
        #[cfg(target_arch = "x86_64")]
        use std::arch::x86_64::{
            __m256i, _mm256_add_epi32, _mm256_and_si256, _mm256_castsi256_ps, _mm256_cmpgt_epi32,
            _mm256_load_si256, _mm256_movemask_ps, _mm256_set1_epi32, _mm256_srai_epi32,
            _mm256_store_si256, _mm256_sub_epi32,
        };

        self.materialize_ring();
        // Runtime states are canonical: 0 <= each slot < 2^31-1. Thus subtraction
        // fits i32; adding M to negative lanes exactly matches the scalar Sample.
        // r[n] = r[n-55] - r[n-24] (mod M), so at most 24 consecutive outputs
        // may be computed without dependencies within that batch. We also stop
        // at physical ring boundaries so every vector load/store stays in bounds.
        let modulus = _mm256_set1_epi32(i32::MAX);
        let cutoff = _mm256_set1_epi32(bound + 1);
        let mut bits = 0_u32;
        let mut produced = 0;
        while produced < 32 {
            let p = if self.inext == 55 { 1 } else { self.inext + 1 };
            let q = if self.inextp == 55 {
                1
            } else {
                self.inextp + 1
            };
            let run = (32 - produced).min(24).min(56 - p).min(56 - q);
            let vector_count = run / 8 * 8;
            for offset in (0..vector_count).step_by(8) {
                let mut left_lanes = Avx2Lanes([0; 8]);
                let mut right_lanes = Avx2Lanes([0; 8]);
                let mut result_lanes = Avx2Lanes([0; 8]);
                // SAFETY: run is bounded by both ring ends, so each copy covers
                // eight valid i32 slots (32 bytes). Locals never overlap the ring.
                // Avx2Lanes provides 32-byte alignment for each vector access.
                // Both operands are copied before the ring is updated, with no
                // dependency within the run (run <= 24). The caller checked AVX2.
                let mask = unsafe {
                    std::ptr::copy_nonoverlapping(
                        self.seed_array.as_ptr().add(p + offset),
                        left_lanes.0.as_mut_ptr(),
                        8,
                    );
                    std::ptr::copy_nonoverlapping(
                        self.seed_array.as_ptr().add(q + offset),
                        right_lanes.0.as_mut_ptr(),
                        8,
                    );
                    let left = _mm256_load_si256(std::ptr::from_ref(&left_lanes).cast::<__m256i>());
                    let right =
                        _mm256_load_si256(std::ptr::from_ref(&right_lanes).cast::<__m256i>());
                    let difference = _mm256_sub_epi32(left, right);
                    let correction = _mm256_and_si256(_mm256_srai_epi32(difference, 31), modulus);
                    let raw = _mm256_add_epi32(difference, correction);
                    _mm256_store_si256(
                        std::ptr::from_mut(&mut result_lanes).cast::<__m256i>(),
                        raw,
                    );
                    std::ptr::copy_nonoverlapping(
                        result_lanes.0.as_ptr(),
                        self.seed_array.as_mut_ptr().add(p + offset),
                        8,
                    );
                    _mm256_movemask_ps(_mm256_castsi256_ps(_mm256_cmpgt_epi32(cutoff, raw)))
                };
                bits |= (mask as u32) << (produced + offset);
            }
            if vector_count != 0 {
                self.inext = p + vector_count - 1;
                self.inextp = q + vector_count - 1;
                produced += vector_count;
                self.draw_position = self.draw_position.saturating_add(vector_count as u64);
            }
            // At most seven scalar values per ring boundary; then vectors resume.
            for _ in vector_count..run {
                bits |= u32::from(self.next_raw() <= bound) << produced;
                produced += 1;
            }
        }
        bits
    }
    pub fn next_range(&mut self, mut mi: i32, mut ma: i32) -> i32 {
        // Preserve the existing scanner API: reversed bounds are swapped.
        // The supplied Mono overload instead throws for reversed bounds.
        if mi > ma {
            std::mem::swap(&mut mi, &mut ma);
        }
        let diff = i64::from(ma) - i64::from(mi);
        if diff <= 1 {
            return mi;
        }
        // C# Next(int min, int max) = (int)((ulong)((uint)(Sample() * diff)) + (ulong)((long)min))
        // Use wrapping unsigned arithmetic to avoid debug-mode overflow panics
        (u64::from((self.next_double() * diff as f64) as u32).wrapping_add(i64::from(mi) as u64))
            as i32
    }
    pub fn next_float(&mut self, min: f32, max: f32) -> f32 {
        self.next_double() as f32 * (max - min) + min
    }
    pub fn random_rate(&mut self, rate: f32) -> bool {
        (self.next_double() as f32) <= rate
    }
}

#[inline]
pub(super) fn threshold_bound(level: i32) -> i32 {
    if level < 0 {
        -1
    } else if level >= 99 {
        i32::MAX - 1
    } else {
        // floor((raw * RECIP) * 100) <= level iff raw <= this integer bound.
        // The closest nonintegral boundary is 1/M away, much greater than f64
        // rounding here. Keep the original floating path for general ranges.
        (((i64::from(level) + 1) * i64::from(i32::MAX) - 1) / 100) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::{threshold_bound, DotNetRandom};

    fn assert_lazy_matches(lazy: &DotNetRandom, sequential: &DotNetRandom) {
        assert!(lazy.ring_dirty);
        assert_eq!(lazy.diagnostic_state(), sequential.diagnostic_state());
        assert_eq!(lazy, sequential);
    }

    fn assert_batch_materializes(lazy: &mut DotNetRandom, sequential: &mut DotNetRandom) {
        let mut portable = lazy.clone();
        let portable_bits = portable.next_threshold_mask_32_scalar(33);
        let dispatched_bits = lazy.next_threshold_mask_32(33);
        let expected_bits = sequential.next_threshold_mask_32(33);
        assert_eq!(portable_bits, expected_bits);
        assert_eq!(dispatched_bits, expected_bits);
        assert!(!portable.ring_dirty);
        assert!(!lazy.ring_dirty);
        assert_eq!(&portable, sequential);
        assert_eq!(lazy, sequential);
    }

    #[test]
    fn lazy_indexed_runs_materialize_only_at_batch_use() {
        for seed in [0, 10_000, -10_000, 755_583_854, 1_235_545_220, i32::MIN] {
            let mut lazy = DotNetRandom::new(seed);
            let mut sequential = lazy.clone();
            for count in [128, 129, 4096, 777, 8192] {
                lazy.advance_indexed(count);
                sequential.advance(count);
                assert_lazy_matches(&lazy, &sequential);
                // Read-only diagnostics and direct-index queries must not
                // cause materialization; the next jump can still merge.
                assert!(lazy.ring_dirty);
                assert_eq!(
                    lazy.indexed_threshold_at(lazy.draw_position(), 33),
                    Some(sequential.clone().next_range(0, 100) <= 33)
                );
            }
            assert_batch_materializes(&mut lazy, &mut sequential);
            lazy.advance_indexed(2000);
            sequential.advance(2000);
            assert_eq!(lazy.next_double(), sequential.next_double());
            assert_lazy_matches(&lazy, &sequential);
            lazy.advance_indexed(3000);
            sequential.advance(3000);
            lazy.advance_indexed(7);
            sequential.advance(7);
            assert_lazy_matches(&lazy, &sequential);
        }
    }

    const SEEDS: [i32; 21] = [
        0,
        1,
        -1,
        161_803_397,
        161_803_398,
        161_803_399,
        321_251_673,
        17_547_620,
        755_583_854,
        -321_251_673,
        614_279_151,
        1_235_228_408,
        1_235_545_220,
        1_333_158_113,
        -614_279_151,
        -1_235_228_408,
        -1_235_545_220,
        -1_333_158_113,
        i32::MAX,
        -i32::MAX,
        i32::MIN,
    ];

    // Independent widened-arithmetic transcription of the supplied C# source.
    // Unlike the production code it does not implement wrap by wrapping_sub.
    #[derive(Clone, Debug)]
    struct Reference {
        a: [i32; 56],
        p: usize,
        q: usize,
    }
    fn narrow(value: i64) -> i32 {
        let reduced = if value > i64::from(i32::MAX) {
            value - 4_294_967_296
        } else if value < i64::from(i32::MIN) {
            value + 4_294_967_296
        } else {
            value
        };
        reduced as i32
    }
    impl Reference {
        fn new(seed: i32) -> Self {
            assert_ne!(seed, i32::MIN, "C# Math.Abs(int.MinValue) throws");
            let absolute = i64::from(seed).abs();
            let mut r = Self {
                a: [0; 56],
                p: 0,
                q: 31,
            };
            let mut d = narrow(161_803_398 - absolute);
            let mut next = 1;
            r.a[55] = d;
            for i in 1..55 {
                let k = 21 * i % 55;
                r.a[k] = next;
                next = narrow(i64::from(d) - i64::from(next));
                if next < 0 {
                    next += i32::MAX;
                }
                d = r.a[k];
            }
            for _ in 0..4 {
                for k in 1..56 {
                    r.a[k] = narrow(i64::from(r.a[k]) - i64::from(r.a[1 + (k + 30) % 55]));
                    if r.a[k] < 0 {
                        r.a[k] += i32::MAX;
                    }
                }
            }
            r
        }
        fn raw(&mut self) -> i32 {
            self.p = self.p % 55 + 1;
            self.q = self.q % 55 + 1;
            let mut value = narrow(i64::from(self.a[self.p]) - i64::from(self.a[self.q]));
            if value < 0 {
                value += i32::MAX;
            }
            self.a[self.p] = value;
            value
        }
        fn state(&self) -> ([i32; 56], usize, usize) {
            (self.a, self.p, self.q)
        }
    }

    #[test]
    fn scalar_matches_independent_reference_and_complete_state() {
        for seed in SEEDS {
            let mut actual = DotNetRandom::new(seed);
            // Preserve the existing Rust API's saturating_abs convention.
            let mut reference = Reference::new(if seed == i32::MIN { i32::MAX } else { seed });
            assert_eq!(actual.diagnostic_state(), reference.state(), "seed={seed}");
            for _ in 0..4096 {
                let raw = reference.raw();
                assert_eq!(
                    actual.next_double().to_bits(),
                    (f64::from(raw) * 4.656_612_875_245_797E-10).to_bits()
                );
                assert_eq!(actual.diagnostic_state(), reference.state(), "seed={seed}");
            }
        }
    }

    #[test]
    fn fixed_next_100_vectors_from_supplied_mono_reference() {
        for (seed, expected) in [
            (321_251_673, [66, 40, 38, 39, 0, 83, 63, 32, 76, 36]),
            (17_547_620, [83, 24, 78, 28, 56, 56, 14, 29, 83, 12]),
            (755_583_854, [56, 54, 4, 88, 28, 92, 18, 14, 19, 65]),
        ] {
            let mut r = DotNetRandom::new(seed);
            for value in expected {
                assert_eq!(r.next_range(0, 100), value);
            }
        }
    }

    #[test]
    fn integer_threshold_matches_original_float_at_every_boundary() {
        for level in -1..=100 {
            let bound = threshold_bound(level);
            for raw in [0, 1, i32::MAX - 2, i32::MAX - 1] {
                let original = ((f64::from(raw) * 4.656_612_875_245_797E-10) * 100.0) as i32;
                assert_eq!(original <= level, raw <= bound);
            }
            for delta in -3_i64..=3 {
                let raw = i64::from(bound) + delta;
                if (0..i64::from(i32::MAX)).contains(&raw) {
                    let original = (((raw as f64) * 4.656_612_875_245_797E-10) * 100.0) as i32;
                    assert_eq!(
                        original <= level,
                        raw <= i64::from(bound),
                        "level={level}, raw={raw}"
                    );
                }
            }
        }
    }

    #[test]
    fn batch_and_peek_match_original_calls_at_every_ring_position() {
        let levels = [
            i32::MIN,
            -1,
            0,
            4,
            20,
            33,
            50,
            86,
            90,
            98,
            99,
            100,
            i32::MAX,
        ];
        for seed in SEEDS {
            let mut anchor = DotNetRandom::new(seed);
            // 0 is the initial pointer, then every physical ring position 1..55.
            for position in 0..=55 {
                for level in levels {
                    assert_batch_matches_original(&anchor, seed, position, level);
                }
                anchor.next_raw();
            }
        }
    }

    fn assert_batch_matches_original(
        anchor: &DotNetRandom,
        seed: i32,
        position: usize,
        level: i32,
    ) {
        let mut original = anchor.clone();
        let mut expected = 0_u32;
        for bit in 0..32 {
            expected |= u32::from(original.next_range(0, 100) <= level) << bit;
        }
        let mut portable = anchor.clone();
        assert_eq!(
            portable.threshold_mask_32_scalar(threshold_bound(level)),
            expected
        );
        assert_eq!(portable, original);
        let mut dispatched = anchor.clone();
        assert_eq!(
            dispatched.next_threshold_mask_32(level),
            expected,
            "seed={seed}, position={position}, level={level}"
        );
        assert_eq!(dispatched, original);
        let peek_source = anchor.clone();
        assert_eq!(peek_source.peek_threshold_mask_32(level), expected);
        assert_eq!(&peek_source, anchor);
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        assert_avx2_matches_original(anchor, &original, level, expected);
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn assert_avx2_matches_original(
        anchor: &DotNetRandom,
        original: &DotNetRandom,
        level: i32,
        expected: u32,
    ) {
        if std::is_x86_feature_detected!("avx2") {
            let mut explicit = anchor.clone();
            // SAFETY: AVX2 support was checked immediately above.
            let mask = unsafe { explicit.threshold_mask_32_avx2(threshold_bound(level)) };
            assert_eq!(mask, expected);
            assert_eq!(&explicit, original);
        }
    }

    #[test]
    fn advance_and_checkpoint_tails_preserve_the_complete_state() {
        for seed in SEEDS {
            for offset in [0, 1, 23, 24, 31, 32, 54, 55, 56, 109] {
                let mut anchor = DotNetRandom::new(seed);
                for _ in 0..offset {
                    anchor.next_raw();
                }
                for count in [
                    0, 1, 2, 7, 8, 23, 24, 31, 32, 33, 54, 55, 56, 63, 64, 65, 127, 255,
                ] {
                    let mut scalar = anchor.clone();
                    for _ in 0..count {
                        scalar.next_raw();
                    }
                    let mut actual = anchor.clone();
                    actual.advance(count);
                    assert_eq!(
                        actual, scalar,
                        "seed={seed}, offset={offset}, count={count}"
                    );
                }
            }
        }
    }

    #[test]
    fn existing_range_and_seed_edge_conventions_are_unchanged() {
        assert_eq!(DotNetRandom::new(i32::MIN), DotNetRandom::new(i32::MAX));
        let ranges = [
            (0, 0),
            (0, 1),
            (i32::MIN, i32::MIN + 1),
            (i32::MAX - 1, i32::MAX),
            (5, 5),
            (100, 0),
            (7, -3),
            (i32::MIN, i32::MAX),
            (i32::MIN, 0),
            (0, i32::MAX),
            (i32::MAX, i32::MIN),
        ];
        for seed in SEEDS {
            let mut actual = DotNetRandom::new(seed);
            let mut reference = Reference::new(if seed == i32::MIN { i32::MAX } else { seed });
            for (lo, hi) in ranges.into_iter().cycle().take(110) {
                let (lo, hi) = (lo.min(hi), lo.max(hi));
                let width = i64::from(hi) - i64::from(lo);
                let expected = if width <= 1 {
                    lo
                } else {
                    let offset = ((f64::from(reference.raw()) * 4.656_612_875_245_797E-10)
                        * (width as f64)) as u32;
                    (u64::from(offset).wrapping_add(i64::from(lo) as u64)) as i32
                };
                // Reverse every other range to verify the pre-existing swap behavior.
                assert_eq!(actual.next_range(hi, lo), expected);
                assert_eq!(actual.diagnostic_state(), reference.state());
            }
        }
    }

    #[test]
    fn indexed_cursor_tracks_mixed_apis_and_checkpoint_clones() {
        for seed in SEEDS {
            assert_indexed_cursor_for_seed(seed);
        }
    }

    fn assert_indexed_cursor_for_seed(seed: i32) {
        let mut actual = DotNetRandom::new(seed);
        actual.next_range(0, 0);
        actual.next_range(0, 1);
        assert_eq!(actual.draw_position(), 0);
        actual.next_range(3, -3);
        actual.next_double();
        actual.next_float(-2.0, 3.0);
        actual.random_rate(0.3);
        assert_eq!(actual.draw_position(), 4);
        let before_peek = actual.clone();
        actual.peek_threshold_mask_32(33);
        assert_eq!(actual, before_peek);
        actual.next_threshold_mask_32(33);
        actual.advance(9);
        actual.next_threshold_mask_32_scalar(90);
        assert_eq!(actual.draw_position(), 77);
        let mut sequential = actual.clone();
        assert_indexed_thresholds_preserve_position(&actual, &mut sequential);
        actual.advance_indexed(128);
        sequential.advance(64);
        assert_eq!(actual, sequential);
        assert_eq!(actual.draw_position(), 205);
    }

    fn assert_indexed_thresholds_preserve_position(
        actual: &DotNetRandom,
        sequential: &mut DotNetRandom,
    ) {
        assert!(actual.indexed_available(10000));
        for index in 77..141 {
            let expected = sequential.next_range(0, 100) <= 33;
            assert_eq!(actual.indexed_threshold_at(index, 33), Some(expected));
        }
        assert_eq!(actual.draw_position(), 77);
    }

    #[test]
    fn operation_readers_preserve_seed_and_preflight_limits() -> Result<(), &'static str> {
        for seed in SEEDS {
            let mut actual = DotNetRandom::new(seed);
            actual.advance(77);
            let mut sequential = actual.clone();
            let reader = actual.indexed_reader(64).ok_or("preflight fits table")?;
            // The view remains valid even after its generator advances.
            actual.advance_indexed(4096);
            for index in 77..141 {
                let expected = sequential.next_range(0, 100);
                for level in [-1, 0, 1, 33, 98, 99, 100] {
                    assert_eq!(
                        reader.raw_at(index) <= threshold_bound(level),
                        expected <= level,
                        "seed={seed}, index={index}, level={level}"
                    );
                }
            }
        }
        let end = super::super::indexed_rng::MAX_DRAWS;
        let mut actual = DotNetRandom::new(i32::MAX);
        actual.advance_indexed(end - 1);
        let reader = actual.indexed_reader(1).ok_or("last sample is supported")?;
        assert_eq!(reader.raw_at(end as u64 - 1), actual.next_raw());
        assert!(actual.indexed_reader(0).is_some());
        assert!(actual.indexed_reader(1).is_none());
        actual.draw_position = u64::MAX;
        assert!(actual.indexed_reader(1).is_none());
        Ok(())
    }

    #[test]
    fn indexed_jumps_and_table_end_fallback_preserve_complete_state() {
        for seed in SEEDS {
            for prefix in [0, 1, 31, 54, 55, 56, 1000] {
                assert_indexed_jumps_from_prefix(seed, prefix);
            }
        }
        assert_table_end_fallback_preserves_complete_state();
    }

    fn assert_table_end_fallback_preserves_complete_state() {
        let end = super::super::indexed_rng::MAX_DRAWS;
        let mut sequential = DotNetRandom::new(1_235_545_220);
        let mut indexed = sequential.clone();
        // Validate the last supported state against actual sequential consumption.
        for _ in 0..end {
            sequential.next_double();
        }
        indexed.advance_indexed(end);
        assert_eq!(indexed, sequential);
        assert!(!indexed.indexed_available(1));
        assert_eq!(indexed.indexed_threshold_at(end as u64, 33), None);
        assert_eq!(indexed.indexed_threshold_at(u64::MAX, 100), None);
        let before = indexed.clone();
        assert_eq!(indexed.indexed_threshold_at(u64::MAX, 33), None);
        assert_eq!(indexed, before);
        // A jump crossing the table end must fall back for its entire duration.
        indexed.advance_indexed(257);
        sequential.advance(257);
        assert_eq!(indexed, sequential);
        assert_eq!(indexed.draw_position(), (end + 257) as u64);
    }

    fn assert_indexed_jumps_from_prefix(seed: i32, prefix: usize) {
        let mut anchor = DotNetRandom::new(seed);
        anchor.advance(prefix);
        for count in [0, 1, 54, 55, 127, 128, 129, 550, 4096] {
            let mut sequential = anchor.clone();
            let mut indexed = anchor.clone();
            sequential.advance(count);
            indexed.advance_indexed(count);
            assert_eq!(
                indexed, sequential,
                "seed={seed}, prefix={prefix}, count={count}"
            );
        }
    }
}
