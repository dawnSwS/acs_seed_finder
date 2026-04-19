#[derive(Clone, Copy, PartialEq, Eq, Hash)] pub enum RandomType { EmNone = 0 }
pub struct DotNetRandom { inext: usize, inextp: usize, seed_array: [i32; 56] }

impl DotNetRandom {
    pub fn new(seed: i32) -> Self {        
        let mut r = Self { inext: 0, inextp: 31, seed_array: [0; 56] };
        let mut mj = 161803398 - seed.checked_abs().unwrap_or(i32::MAX);
        r.seed_array[55] = mj; let mut mk = 1;
        for i in 1..55 { let ii = (21 * i) % 55; r.seed_array[ii] = mk; mk = mj - mk; if mk < 0 { mk += i32::MAX; } mj = r.seed_array[ii]; }
        for _ in 1..5 { for k in 1..56 { r.seed_array[k] -= r.seed_array[1 + (k + 30) % 55]; if r.seed_array[k] < 0 { r.seed_array[k] += i32::MAX; } } }
        r
    }
    
    pub fn next_range(&mut self, mut min: i32, mut max: i32) -> i32 {
        if min > max { std::mem::swap(&mut min, &mut max); }
        let diff = (max as i64) - (min as i64); if diff <= 1 { return min; }
        self.inext = (self.inext + 1) % 56; if self.inext == 0 { self.inext = 1; }
        self.inextp = (self.inextp + 1) % 56; if self.inextp == 0 { self.inextp = 1; }
        let mut num = self.seed_array[self.inext] - self.seed_array[self.inextp];
        if num < 0 { num += i32::MAX; } self.seed_array[self.inext] = num;
        ((num as f64 * 4.656612875245797E-10 * diff as f64) as u32).wrapping_add(min as u32) as i32
    }

    pub fn next_double(&mut self) -> f64 {
        self.inext = (self.inext + 1) % 56; if self.inext == 0 { self.inext = 1; }
        self.inextp = (self.inextp + 1) % 56; if self.inextp == 0 { self.inextp = 1; }
        let mut num = self.seed_array[self.inext] - self.seed_array[self.inextp];
        if num == i32::MAX { num -= 1; } if num < 0 { num += i32::MAX; }
        self.seed_array[self.inext] = num; (num as f64) * 4.6566128752457969E-10
    }

    pub fn next_range_strict(&mut self, mut min: i32, mut max: i32) -> i32 {
        if min > max { std::mem::swap(&mut min, &mut max); }
        ((self.next_double() * ((max as i64) - (min as i64)) as f64) as i64 + min as i64) as i32
    }

    pub fn next_float(&mut self, min: f32, max: f32) -> f32 { (self.next_double() as f32) * (max - min) + min }
    pub fn random_rate(&mut self, rate: f32) -> bool { (self.next_double() as f32) < rate }
    pub fn advance(&mut self, steps: usize) { (0..steps).for_each(|_| { self.next_double(); }); }
    
    pub fn box_muller_trap(&mut self) -> f32 {
        std::iter::repeat_with(|| (self.next_double() as f32 * 2.0 - 1.0, self.next_double() as f32 * 2.0 - 1.0))
            .map(|(n1, n2)| (n1, n1 * n1 + n2 * n2)).find(|&(_, n3)| n3 > 0.0 && n3 < 1.0)
            .map(|(n1, n3)| n1 * ((-2.0 * n3.ln()) / n3).sqrt()).unwrap()
    }
}

pub struct GMathUtl { sys_random: DotNetRandom }
impl GMathUtl {
    pub fn new(seed: i32) -> Self { Self { sys_random: DotNetRandom::new(seed) } }
    #[inline(always)] pub fn random_range_int(&mut self, min: i32, max: i32, _: RandomType, _: &str) -> i32 { self.sys_random.next_range(min, max) }
}

pub fn string_hash(s: &str) -> i32 { s.encode_utf16().fold(0, |n, c| n.wrapping_shl(5).wrapping_sub(n).wrapping_add(c as i32)) }

pub fn find_chinese_collision(seed: i32) -> Option<String> {
    let offset = "玄黎曰".encode_utf16().fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32)).wrapping_mul(31_u32.pow(5));
    let mut rem = (seed as u32).wrapping_sub(offset).wrapping_sub(2450903684);
    String::from_utf16(&[923521, 29791, 961, 31, 1].map(|d| { let v = rem / d; rem %= d; (0x7384 + v) as u16 })).ok().map(|s| format!("玄黎曰{s}"))
}