//! Exact random access to the supplied legacy Mono random stream.
//!
//! For seed magnitude s and one-based output n, the raw value is
//! (A[n] * s + B[n] + e1(s) * U[n] + e2(s) * V[n]) mod (2^31-1).
//! The two family bits account for unchecked i32 overflows in the first warm
//! round at slots 24 and 55. Ignoring these corrections changes valid seeds.
//! All states are canonical after warm round two, including the four seeds
//! with a temporary -1 or M representation; subsequent modulo arithmetic is exact.
//!
//! The shared table has a fixed memory cap. Queries outside it return None;
//! callers must retain their sequential implementation as the fallback.
#![allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::indexing_slicing
)]

use std::sync::OnceLock;

const M: i64 = i32::MAX as i64;
const C: i64 = 161_803_398;
pub const MAX_DRAWS: usize = 2_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableInfo {
    pub max_draws: usize,
    pub coefficient_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Seed {
    magnitude: u32,
    family: u32,
}

impl Seed {
    #[cfg(feature = "gpu")]
    pub(crate) fn gpu_words(self, seed: i32) -> [u32; 4] {
        [seed as u32, self.magnitude, self.family, 0]
    }

    pub(crate) fn new(seed: i32) -> Self {
        // Match the existing Rust scanner's saturating_abs convention, including
        // its documented int.MinValue extension beyond the supplied C# API.
        let magnitude = seed.saturating_abs() as u32;
        let h = i64::from(magnitude) - C;
        let first = (34 + 21 * h).rem_euclid(M) + h > M;
        let second = (997_453_707 * h + 286_696_116).rem_euclid(M) + h > M + 1;
        Self {
            magnitude,
            family: u32::from(first) | (u32::from(second) << 1),
        }
    }
}

#[derive(Clone, Copy)]
struct Coefficient {
    a: u32,
    b: u32,
    u: u32,
    v: u32,
}

impl Coefficient {
    #[inline]
    fn evaluate(self, seed: Seed) -> i32 {
        // Even with both corrections the sum is at most 2^62-4, so no u64
        // overflow occurs and one Mersenne fold plus subtraction is sufficient.
        let mut value = u64::from(self.a) * u64::from(seed.magnitude) + u64::from(self.b);
        if seed.family & 1 != 0 {
            value += u64::from(self.u);
        }
        if seed.family & 2 != 0 {
            value += u64::from(self.v);
        }
        reduce_mersenne(value)
    }
}

/// Exact reduction for the coefficient sum, whose upper bound is 2^62-4.
#[inline]
fn reduce_mersenne(value: u64) -> i32 {
    // 2^31 == 1 (mod M). The two 31-bit parts sum to less than 2*M:
    // equality would require value == 2^62-1, outside the coefficient bound.
    let folded = (value & M as u64) + (value >> 31);
    if folded >= M as u64 {
        (folded - M as u64) as i32
    } else {
        folded as i32
    }
}

struct LinearStream {
    a: [i64; 56],
    p: usize,
    q: usize,
}

#[inline]
fn subtract_mod(left: i64, right: i64) -> i64 {
    let difference = left - right;
    if difference < 0 {
        difference + M
    } else {
        difference
    }
}

impl LinearStream {
    fn new(seed_basis: i64, impulse: u8) -> Self {
        let mut a = [0; 56];
        let mut d = (C - seed_basis).rem_euclid(M);
        let mut next = 1;
        a[55] = d;
        for i in 1..55 {
            let k = 21 * i % 55;
            a[k] = next;
            next = subtract_mod(d, next);
            d = a[k];
        }
        if impulse != 0 {
            a.fill(0);
        }
        for round in 0..4 {
            for k in 1..56 {
                a[k] = subtract_mod(a[k], a[1 + (k + 30) % 55]);
                if round == 0 && k == 24 && impulse == 1 {
                    a[k] = subtract_mod(a[k], 2);
                }
                if round == 0 && k == 55 && impulse == 2 {
                    a[k] = (a[k] + 2) % M;
                }
            }
        }
        Self { a, p: 0, q: 31 }
    }

    fn next(&mut self) -> i64 {
        self.p = self.p % 55 + 1;
        self.q = self.q % 55 + 1;
        self.a[self.p] = subtract_mod(self.a[self.p], self.a[self.q]);
        self.a[self.p]
    }
}

struct Table {
    // Zero-based: coefficients[0] describes the first consumed sample.
    coefficients: Vec<Coefficient>,
}

impl Table {
    fn build() -> Option<Self> {
        let mut coefficients = Vec::new();
        // Allocation failure leaves indexing disabled; sequential generation
        // remains valid and is selected by the caller's preflight/fallback.
        coefficients.try_reserve_exact(MAX_DRAWS).ok()?;
        let mut base = LinearStream::new(0, 0);
        let mut one = LinearStream::new(1, 0);
        let mut u = LinearStream::new(0, 1);
        let mut v = LinearStream::new(0, 2);
        for _ in 0..MAX_DRAWS {
            let b = base.next();
            coefficients.push(Coefficient {
                a: subtract_mod(one.next(), b) as u32,
                b: b as u32,
                u: u.next() as u32,
                v: v.next() as u32,
            });
        }
        Some(Self { coefficients })
    }
}

static TABLE: OnceLock<Option<Table>> = OnceLock::new();

fn table() -> Option<&'static Table> {
    TABLE.get_or_init(Table::build).as_ref()
}

/// Build the immutable shared coefficient table once, outside benchmark timing.
/// Returns None if allocation fails; no map or RNG state is changed.
pub fn prepare() -> Option<TableInfo> {
    let table = table()?;
    Some(TableInfo {
        max_draws: table.coefficients.len(),
        coefficient_bytes: table.coefficients.len() * std::mem::size_of::<Coefficient>(),
    })
}

/// Exact shared coefficients for the GPU; one-time initialization, never per seed.
#[cfg(feature = "gpu")]
pub(crate) fn gpu_coefficients() -> Option<Vec<[u32; 4]>> {
    Some(
        table()?
            .coefficients
            .iter()
            .map(|c| [c.a, c.b, c.u, c.v])
            .collect(),
    )
}

pub(crate) fn available(end_exclusive: u64) -> bool {
    end_exclusive <= MAX_DRAWS as u64 && table().is_some()
}

/// Immutable view captured once for a preflighted indexed operation.
/// It owns no RNG state and therefore does not borrow the caller's generator.
pub(super) struct Reader {
    coefficients: &'static [Coefficient],
    seed: Seed,
}

impl Reader {
    /// The caller's preflight must cover the requested absolute sample index.
    /// Check the bound before narrowing so invalid indices fail on every target.
    #[inline]
    pub(super) fn raw_at(&self, zero_based: u64) -> i32 {
        // Check before narrowing so an invalid u64 index cannot wrap to a valid
        // slot on 32-bit targets. A valid index is below the table's usize length.
        assert!(
            zero_based < self.coefficients.len() as u64,
            "preflight must cover the requested sample index"
        );
        let index = zero_based as usize;
        self.coefficients[index].evaluate(self.seed)
    }
}

pub(super) fn reader(seed: Seed, end_exclusive: u64) -> Option<Reader> {
    if end_exclusive > MAX_DRAWS as u64 {
        return None;
    }
    Some(Reader {
        coefficients: &table()?.coefficients[..end_exclusive as usize],
        seed,
    })
}

/// Read a zero-based output position without consuming the caller's RNG.
pub(crate) fn raw_at(seed: Seed, zero_based: u64) -> Option<i32> {
    if zero_based >= MAX_DRAWS as u64 {
        return None;
    }
    table()?
        .coefficients
        .get(zero_based as usize)
        .map(|coefficient| coefficient.evaluate(seed))
}

/// Reconstruct the exact physical ring after `consumed` samples.
/// Before sample 55 not every physical slot has been overwritten, so callers
/// must use sequential advancement for those targets.
pub(crate) fn state_after(seed: Seed, consumed: u64) -> Option<([i32; 56], usize, usize)> {
    if consumed < 55 || consumed > MAX_DRAWS as u64 {
        return None;
    }
    let table = table()?;
    let mut state = [0; 56];
    let start = consumed as usize - 55;
    for (index, coefficient) in table.coefficients[start..consumed as usize]
        .iter()
        .enumerate()
    {
        state[(start + index) % 55 + 1] = coefficient.evaluate(seed);
    }
    Some((
        state,
        (consumed as usize - 1) % 55 + 1,
        (consumed as usize + 30) % 55 + 1,
    ))
}

#[cfg(test)]
mod tests {
    use super::{prepare, raw_at, reduce_mersenne, state_after, Coefficient, Seed, M, MAX_DRAWS};
    use crate::core::rng::DotNetRandom;

    #[test]
    fn mersenne_fold_matches_remainder_at_reduction_boundaries() {
        let modulus = M as u64;
        let max_value = (1_u64 << 62) - 4;
        // Exercise both sides of modulus multiples, the 31-bit carry, and the
        // largest legal sum. Large quotients also force the final subtraction.
        for quotient in [0, 1, 2, 3, modulus / 2, modulus - 1, modulus, modulus + 1] {
            for remainder in [0, 1, 2, modulus / 2, modulus - 2, modulus - 1] {
                let value = quotient * modulus + remainder;
                if value <= max_value {
                    assert_eq!(
                        reduce_mersenne(value),
                        (value % modulus) as i32,
                        "value={value}"
                    );
                }
            }
        }
        for value in [
            (1_u64 << 31) - 1,
            1_u64 << 31,
            (1_u64 << 31) + 1,
            max_value - 1,
            max_value,
        ] {
            assert_eq!(
                reduce_mersenne(value),
                (value % modulus) as i32,
                "value={value}"
            );
        }
    }

    #[test]
    fn coefficient_evaluation_matches_remainder_for_all_seed_families() {
        let mut observed_families = [false; 4];
        let components = [0, 1, M as u32 / 2, M as u32 - 2, M as u32 - 1];
        for seed_value in [
            0,
            1,
            -1,
            161_803_398,
            755_583_854,
            1_235_228_408,
            1_235_545_220,
            i32::MAX,
            i32::MIN,
        ] {
            let seed = Seed::new(seed_value);
            observed_families[seed.family as usize] = true;
            for a in components {
                for b in components {
                    for u in components {
                        for v in components {
                            let coefficient = Coefficient { a, b, u, v };
                            let value = u64::from(a) * u64::from(seed.magnitude)
                                + u64::from(b)
                                + u64::from(seed.family & 1) * u64::from(u)
                                + u64::from((seed.family >> 1) & 1) * u64::from(v);
                            assert_eq!(
                                coefficient.evaluate(seed),
                                (value % M as u64) as i32,
                                "seed={seed_value}, coefficients=({a},{b},{u},{v})"
                            );
                        }
                    }
                }
            }
        }
        assert_eq!(observed_families, [true; 4]);
    }

    #[test]
    fn indexed_values_and_reconstructed_states_match_the_sequential_stream(
    ) -> Result<(), &'static str> {
        let metadata = prepare().ok_or("test coefficient table allocation")?;
        assert_eq!(metadata.coefficient_bytes, MAX_DRAWS * 16);
        let seeds = [
            0,
            1,
            -1,
            161_803_397,
            161_803_398,
            161_803_399,
            321_251_673,
            17_547_620,
            755_583_854,
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
        for seed in seeds {
            let descriptor = Seed::new(seed);
            let mut sequential = DotNetRandom::new(seed);
            for index in 0..8192 {
                sequential.next_double();
                let (slots, p, _) = sequential.diagnostic_state();
                assert_eq!(
                    raw_at(descriptor, index),
                    Some(slots[p]),
                    "seed={seed}, index={index}"
                );
                if index >= 54 && (index < 120 || index % 997 == 0) {
                    assert_eq!(
                        state_after(descriptor, index + 1),
                        Some(sequential.diagnostic_state())
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn table_limits_return_none_without_mutating_a_generator() {
        let seed = Seed::new(1);
        assert!(raw_at(seed, MAX_DRAWS as u64 - 1).is_some());
        assert_eq!(raw_at(seed, MAX_DRAWS as u64), None);
        assert_eq!(raw_at(seed, u64::MAX), None);
        assert_eq!(state_after(seed, 0), None);
        assert_eq!(state_after(seed, 54), None);
        assert!(state_after(seed, MAX_DRAWS as u64).is_some());
        assert_eq!(state_after(seed, MAX_DRAWS as u64 + 1), None);
    }

    #[test]
    #[should_panic(expected = "preflight must cover the requested sample index")]
    fn operation_reader_rejects_indices_that_would_wrap_on_32_bit_targets() {
        let reader = super::Reader {
            coefficients: &[Coefficient {
                a: 0,
                b: 0,
                u: 0,
                v: 0,
            }],
            seed: Seed::new(1),
        };
        reader.raw_at(u64::from(u32::MAX) + 1);
    }
}
