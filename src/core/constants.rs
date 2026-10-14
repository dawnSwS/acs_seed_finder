use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CType {
    AllTrue,
    NoBorn,
    CheckCon,
    CheckCon2,
    NoLing,
    OnlySoil,
}

pub fn get_base_around_50() -> &'static [(i32, i32)] {
    static B: OnceLock<Vec<(i32, i32)>> = OnceLock::new();
    B.get_or_init(|| {
        let (mut r, mut x, mut y, mut direction, mut side) = (vec![(0, 0)], 0, 0, 1, 1);
        while r.len() < 10_201 {
            for _ in 0..2 {
                for _ in 0..side {
                    match direction {
                        0 => x += 1,
                        1 => y -= 1,
                        2 => x -= 1,
                        _ => y += 1,
                    }
                    r.push((x, y));
                    if r.len() == 10_201 {
                        return r;
                    }
                }
                direction = (direction + 1) % 4;
            }
            side += 1;
        }
        r
    })
}
