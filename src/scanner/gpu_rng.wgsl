// Exact indexed legacy Mono lag-31 RNG. The shared coefficient table and host
// seed descriptor come from core/indexed_rng.rs; there is no private RNG ring.
// Logical draw positions remain sequential wherever map state determines the
// next operation. Constant-result draws can be skipped without losing position.

fn advance_draws(count: u32) {
    if error_code != 0u { return; }
    if count > cfg.max_draws - rng_draws {
        rng_draws = cfg.max_draws;
        error_code = 1u;
        return;
    }
    rng_draws += count;
}

fn raw_at(position: u32) -> u32 {
    rng_queries += 1u;
    let c = coefficients[position];
    // Both factors are at most M=2^31-1; all intermediates below fit u32.
    // In particular mid <= 0xfffe0000. Form exact unsigned 64-bit product.
    let a0 = c.x & 65535u;
    let a1 = c.x >> 16u;
    let s0 = seed_magnitude & 65535u;
    let s1 = seed_magnitude >> 16u;
    let p0 = a0 * s0;
    let mid = (p0 >> 16u) + a0 * s1 + a1 * s0;
    var lo = (p0 & 65535u) | (mid << 16u);
    var hi = a1 * s1 + (mid >> 16u);
    var old = lo;
    lo += c.y;
    hi += select(0u, 1u, lo < old);
    if (seed_family & 1u) != 0u {
        old = lo;
        lo += c.z;
        hi += select(0u, 1u, lo < old);
    }
    if (seed_family & 2u) != 0u {
        old = lo;
        lo += c.w;
        hi += select(0u, 1u, lo < old);
    }
    // Total <= 2^62-4: one Mersenne fold and one subtraction suffice.
    let folded = (lo & 2147483647u) + ((hi << 1u) | (lo >> 31u));
    return select(folded, folded - 2147483647u, folded >= 2147483647u);
}

fn next_raw() -> i32 {
    if error_code != 0u { return 0; }
    if rng_draws >= cfg.max_draws {
        error_code = 1u;
        return 0;
    }
    let raw = raw_at(rng_draws);
    rng_draws += 1u;
    return i32(raw);
}

// floor(raw * span / (2^31-1)), with raw < 2^31-1 and span <= 4096.
fn scale_raw(raw: u32, span: u32) -> u32 {
    let low_product = (raw & 65535u) * span;
    let high_product = (raw >> 16u) * span;
    let product_lo = low_product + (high_product << 16u);
    let carry = select(0u, 1u, product_lo < low_product);
    let product_hi = (high_product >> 16u) + carry;
    let quotient = (product_hi << 1u) | (product_lo >> 31u);
    let remainder = (product_lo & 2147483647u) + quotient;
    return quotient + select(0u, 1u, remainder >= 2147483647u);
}

fn next_range(first: i32, second: i32) -> i32 {
    let lower = min(first, second);
    let upper = max(first, second);
    let span = bitcast<u32>(upper) - bitcast<u32>(lower);
    if span <= 1u || error_code != 0u { return lower; }
    if span > 4096u {
        error_code = 3u;
        return lower;
    }
    return lower + i32(scale_raw(u32(next_raw()), span));
}
