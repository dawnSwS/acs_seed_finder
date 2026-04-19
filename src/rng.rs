#[derive(Clone, Copy, PartialEq, Eq, Hash)] pub enum RandomType { EmNone = 0 }
pub struct DotNetRandom { inext: usize, inextp: usize, seed_array: [i32; 56] }

impl DotNetRandom {
    pub fn new(seed: i32) -> Self {        
        let mut rng = Self { inext: 0, inextp: 31, seed_array: [0; 56] };
        let mut mj = 161803398 - if seed == i32::MIN { i32::MAX } else { seed.abs() };
        rng.seed_array[55] = mj; let mut mk = 1;
        for i in 1..55 { let ii = (21 * i) % 55; rng.seed_array[ii] = mk; mk = mj - mk; if mk < 0 { mk += 2147483647; } mj = rng.seed_array[ii]; }
        for _ in 1..5 { for k in 1..56 { rng.seed_array[k] -= rng.seed_array[1 + (k + 30) % 55]; if rng.seed_array[k] < 0 { rng.seed_array[k] += 2147483647; } } }
        rng
    }
    pub fn next_range(&mut self, mut min_v: i32, mut max_v: i32) -> i32 {
        if min_v > max_v { std::mem::swap(&mut min_v, &mut max_v); }
        let diff = (max_v as i64) - (min_v as i64);
        if diff <= 1 { return min_v; }
        self.inext = if self.inext + 1 >= 56 { 1 } else { self.inext + 1 };
        self.inextp = if self.inextp + 1 >= 56 { 1 } else { self.inextp + 1 };
        let mut num = self.seed_array[self.inext] - self.seed_array[self.inextp];
        if num < 0 { num += 2147483647; } self.seed_array[self.inext] = num;
        ((num as f64 * 4.656612875245797E-10 * diff as f64) as u32).wrapping_add(min_v as u32) as i32
    }
}

pub struct GMathUtl { sys_random: DotNetRandom }
impl GMathUtl {
    pub fn new(seed: i32) -> Self { Self { sys_random: DotNetRandom::new(seed) } }
    #[inline(always)] pub fn random_range_int(&mut self, min: i32, max: i32, _: RandomType, _: &str) -> i32 { self.sys_random.next_range(min, max) }
}

pub fn string_hash(s: &str) -> i32 { s.encode_utf16().fold(0, |num, c| num.wrapping_shl(5).wrapping_sub(num).wrapping_add(c as i32)) }

pub fn find_chinese_collision(target_seed: i32) -> Option<String> {
    let offset = (0..5).fold("玄黎曰".encode_utf16().fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32)), |a, _| a.wrapping_mul(31));
    let mut rem = (target_seed as u32).wrapping_sub(offset).wrapping_sub(2450903684);
    let chars: Vec<u16> = [923521, 29791, 961, 31, 1].iter().map(|&div| { let v = rem / div; rem %= div; (0x7384 + v) as u16 }).collect();
    String::from_utf16(&chars).ok().map(|suffix| format!("玄黎曰{}", suffix))
}