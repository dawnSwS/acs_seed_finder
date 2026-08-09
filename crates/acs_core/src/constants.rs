use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq)]
pub enum CType {
    AllTrue,
    NoBorn,
    CheckCon,
    CheckCon2,
}

pub fn get_base_around_50() -> &'static [(i32, i32)] {
    static B: OnceLock<Vec<(i32, i32)>> = OnceLock::new();
    B.get_or_init(|| {
        let (mut r, mut x, mut y, mut d, mut s) = (vec![(0, 0)], 0, 0, 1, 1);
        while r.len() < 10201 {
            for _ in 0..2 {
                for _ in 0..s {
                    match d {
                        0 => x += 1,
                        1 => y -= 1,
                        2 => x -= 1,
                        _ => y += 1,
                    };
                    r.push((x, y));
                    if r.len() == 10201 {
                        return r;
                    }
                }
                d = (d + 1) % 4;
            }
            s += 1;
        }
        r
    })
}
