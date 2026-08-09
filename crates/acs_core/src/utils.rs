pub fn string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0, |n, c| {
        n.wrapping_shl(5).wrapping_sub(n).wrapping_add(c as i32)
    })
}

pub fn find_chinese_collision(s: i32) -> Option<String> {
    let mut r = (s as u32)
        .wrapping_sub(
            "玄黎曰"
                .encode_utf16()
                .fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32))
                .wrapping_mul(28629151),
        )
        .wrapping_sub(2450903684);
    String::from_utf16(&[923521, 29791, 961, 31, 1].map(|d| {
        let v = r / d;
        r %= d;
        (0x7384 + v) as u16
    }))
    .ok()
    .map(|c| format!("玄黎曰{c}"))
}
