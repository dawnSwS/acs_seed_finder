#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum RandomType {
    EmNone = 0,
    EmJianghu = 5,
}

pub struct GRandom {
    pub seed: u32,
}
impl GRandom {
    pub fn new(seed: u32) -> Self {
        Self { seed }
    }
    pub fn rand(&mut self) -> u32 {
        self.seed = (self.seed as u64)
            .wrapping_mul(1103515245)
            .wrapping_add(12345) as u32;
        self.seed = (self.seed << 16) | (self.seed >> 16);
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
        let ratio = self.rand() as f64 / 4294967295.0;
        let span = c_max.wrapping_sub(c_min).wrapping_add(1);
        let num2 = (ratio * (span as f64)) as i32;
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

        let ratio = self.rand() as f64 / 4294967295.0;
        // Single f64 multiplication, then truncate to f32 (matching C# exactly)
        let num2 = (ratio * ((max - min) as f64)) as f32;

        num2 + min
    }
}

pub struct DotNetRandom {
    inext: usize,
    inextp: usize,
    seed_array: [i32; 56],
}
impl DotNetRandom {
    pub fn new(seed: i32) -> Self {
        let mut r = Self {
            inext: 0,
            inextp: 31,
            seed_array: [0; 56],
        };
        let mut mj = 161803398 - seed.saturating_abs();
        r.seed_array[55] = mj;
        let mut mk = 1;
        for i in 1..55 {
            let ii = (21 * i) % 55;
            r.seed_array[ii] = mk;
            mk = mj - mk;
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
    pub fn next_double(&mut self) -> f64 {
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
        if n == i32::MAX {
            n -= 1;
        }
        if n < 0 {
            n += i32::MAX;
        }
        self.seed_array[self.inext] = n;
        n as f64 * 4.656612875245797e-10
    }
    pub fn next_range(&mut self, mut mi: i32, mut ma: i32) -> i32 {
        if mi > ma {
            std::mem::swap(&mut mi, &mut ma);
        }
        let diff = ma as i64 - mi as i64;
        if diff <= 1 {
            return mi;
        }
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
        // C# Next(int min, int max) = (int)((ulong)((uint)(Sample() * diff)) + (ulong)((long)min))
        // Use wrapping unsigned arithmetic to avoid debug-mode overflow panics
        (((n as f64 * 4.656612875245797e-10 * diff as f64) as u32 as u64)
            .wrapping_add(mi as i64 as u64)) as i32
    }
    pub fn next_float(&mut self, min: f32, max: f32) -> f32 {
        self.next_double() as f32 * (max - min) + min
    }
    pub fn random_rate(&mut self, rate: f32) -> bool {
        (self.next_double() as f32) <= rate
    }
    pub fn box_muller_trap(&mut self) -> f32 {
        loop {
            let n1 = self.next_double() as f32 * 2. - 1.;
            let n2 = self.next_double() as f32 * 2. - 1.;
            let n3 = n1 * n1 + n2 * n2;
            if n3 > 0.0 && n3 < 1.0 {
                return n1 * ((-2. * n3.ln()) / n3).sqrt();
            }
        }
    }
}

pub struct GMathUtl {
    pub sys_random: DotNetRandom,
}
impl GMathUtl {
    pub fn new(s: i32) -> Self {
        Self {
            sys_random: DotNetRandom::new(s),
        }
    }
    #[inline]
    pub fn random_range_int(&mut self, mi: i32, ma: i32, _: RandomType, _: &str) -> i32 {
        self.sys_random.next_range(mi, ma)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helper: standalone LCG+rotate to compute expected values WITHOUT calling GRandom ──
    fn raw_lcg(seed: u32) -> u32 {
        let step1 = (seed as u64).wrapping_mul(1103515245).wrapping_add(12345) as u32;
        (step1 << 16) | (step1 >> 16)
    }

    // ═══════════════════════════════════════════════════════════════
    // A. GRandom gold-sample tests
    // ═══════════════════════════════════════════════════════════════

    /// A.1 – seed 5489 (default), first 3 rand() outputs
    #[test]
    fn grandom_default_seed_first3() {
        let mut rng = GRandom::new(5489);
        // Independent LCG computation (no dependency on GRandom itself)
        let mut s = 5489u32;
        for &expected in &[3210104055u32, 929300796, 4274339420] {
            s = raw_lcg(s);
            assert_eq!(rng.rand(), expected, "mismatch at seed trail");
        }
        // Also verify the seed stored inside matches
        assert_eq!(rng.seed, s);
    }

    /// A.2 – seed=1, rand_range(1,7)
    /// C# GRandom.RandRange(nMin=1, nMax=7) closed interval [1,7].
    /// Rust rand_range(a,b) subtracts 1 internally → C# sees [1,6], span=6.
    /// Hand calculation: u1 = raw_lcg(1) = 2124825030;
    /// ratio = 2124825030/4294967295.0 ≈ 0.49472;
    /// num2 = (ratio * 6) as i32 = 2; num3 = 2+1 = 3; clamp → 3.
    #[test]
    fn grandom_rand_range_seed1() {
        let mut rng = GRandom::new(1);
        let result = rng.rand_range(1, 7);
        assert_eq!(result, 3, "rand_range(1,7) with seed=1 should be 3");
    }

    /// A.3 – degenerate interval rand_range(1,1)
    /// 1) result ∈ {0, 1}
    /// 2) state advances: calling rand_range(1,1) then rand_range(1,1000000000)
    ///    equals calling one rand() then rand_range(1,1000000000)
    #[test]
    fn grandom_degenerate_range_advances_state() {
        // --- path A: degenerate call first ---
        let mut rng_a = GRandom::new(1);
        let degenerate_result = rng_a.rand_range(1, 1);
        assert!(
            degenerate_result == 0 || degenerate_result == 1,
            "rand_range(1,1) must return 0 or 1, got {}",
            degenerate_result
        );
        let after_degenerate = rng_a.rand_range(1, 1_000_000_000);

        // --- path B: manual single rand() then same call ---
        let mut rng_b = GRandom::new(1);
        let _manual_rand = rng_b.rand(); // advance state by exactly one rand()
        let after_manual = rng_b.rand_range(1, 1_000_000_000);

        assert_eq!(
            after_degenerate, after_manual,
            "degenerate rand_range must advance state by exactly one rand()"
        );
        // Gold value from independent LCG (seed=1 → one rand → then rand_range(1,1e9))
        assert_eq!(after_degenerate, 521_602_271);
    }

    /// A.4 – rand_float precision
    /// rand_float(0.0, 0.2) == ((rand() as f64 / 4294967295.0) * 0.2) as f32
    /// Verified with independent computation for seed=42.
    #[test]
    fn grandom_rand_float_precision() {
        let expected_u = raw_lcg(42);
        let expected_f64 = (expected_u as f64 / 4294967295.0) * 0.2;
        let expected_f32 = expected_f64 as f32;

        let mut rng = GRandom::new(42);
        let actual = rng.rand_float(0.0, 0.2);

        assert_eq!(actual, expected_f32,
            "rand_float(0.0,0.2) precision mismatch: got {}, expected {} (bits: got {:#x}, expected {:#x})",
            actual, expected_f32, actual.to_bits(), expected_f32.to_bits());
        // Extra: verify the internal rand() was consumed (state advanced)
        let next = rng.rand();
        assert_eq!(
            next,
            raw_lcg(expected_u),
            "rand() after rand_float should produce the next LCG step"
        );
    }

    // ═══════════════════════════════════════════════════════════════
    // B. DotNetRandom tests
    // ═══════════════════════════════════════════════════════════════

    /// B.1 – large-range next_range does not panic and stays in bounds
    #[test]
    fn dotnet_random_large_range_no_panic() {
        let mut rng = DotNetRandom::new(42);
        for _ in 0..20 {
            let v = rng.next_range(-2_000_000_000, 2_000_000_000);
            assert!(
                v >= -2_000_000_000 && v <= 2_000_000_000,
                "next_range out of bounds: {}",
                v
            );
        }
    }

    /// B.2 – min == max → returns that value, no state consumed
    /// C# Next(min, max): diff ≤ 1 → return min without Sample().
    #[test]
    fn dotnet_random_degenerate_no_state_change() {
        let mut rng1 = DotNetRandom::new(42);
        let _ = rng1.next_range(7, 7); // degenerate, should NOT advance state
        let v1 = rng1.next_range(0, 100);

        let mut rng2 = DotNetRandom::new(42);
        let v2 = rng2.next_range(0, 100); // no degenerate call

        assert_eq!(v1, 66, "expected gold value from seed=42");
        assert_eq!(v2, 66);
        assert_eq!(
            v1, v2,
            "degenerate next_range must not advance DotNetRandom state"
        );
    }

    /// B.3 – determinism: same seed → identical sequence
    #[test]
    fn dotnet_random_determinism() {
        let mut rng1 = DotNetRandom::new(12345);
        let mut rng2 = DotNetRandom::new(12345);
        for i in 0..50 {
            let v1 = rng1.next_range(0, 1000);
            let v2 = rng2.next_range(0, 1000);
            assert_eq!(v1, v2, "determinism failed at step {}", i);
        }
    }
}
