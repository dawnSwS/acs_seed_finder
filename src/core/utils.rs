pub fn string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0, |n, c| {
        n.wrapping_shl(5).wrapping_sub(n).wrapping_add(i32::from(c))
    })
}

// The first quotient is at most 4650 and all later quotients at most 30;
// adding 0x7384 remains in the BMP Chinese range and fits in u16.
#[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
pub fn find_chinese_collision(s: i32) -> Option<String> {
    let mut r = u32::from_ne_bytes(s.to_ne_bytes())
        .wrapping_sub(
            "玄黎曰"
                .encode_utf16()
                .fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(u32::from(c)))
                .wrapping_mul(28_629_151),
        )
        .wrapping_sub(2_450_903_684);
    String::from_utf16(&[923_521, 29_791, 961, 31, 1].map(|d| {
        let v = r / d;
        r %= d;
        (0x7384 + v) as u16
    }))
    .ok()
    .map(|c| format!("玄黎曰{c}"))
}
