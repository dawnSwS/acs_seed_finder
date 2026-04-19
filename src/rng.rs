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
    
    // 用于地图等大量抽取的的高速跳过版本
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

    // 🌟新增：核心系统浮点源，C# Random.Sample() 的 1:1 实现
    pub fn next_double(&mut self) -> f64 {
        self.inext = if self.inext + 1 >= 56 { 1 } else { self.inext + 1 };
        self.inextp = if self.inextp + 1 >= 56 { 1 } else { self.inextp + 1 };
        let mut num = self.seed_array[self.inext] - self.seed_array[self.inextp];
        if num == 2147483647 { num -= 1; } // C# 特有修正
        if num < 0 { num += 2147483647; }
        self.seed_array[self.inext] = num;
        (num as f64) * 4.6566128752457969E-10
    }

    // 🌟新增：专用于 NPC 抽卡的严格刻度版，哪怕可抽范围是 0，也严格吞噬 1 次天道游标
    pub fn next_range_strict(&mut self, mut min_v: i32, mut max_v: i32) -> i32 {
        if min_v > max_v { std::mem::swap(&mut min_v, &mut max_v); }
        let diff = (max_v as i64) - (min_v as i64);
        let num = self.next_double();
        ((num * diff as f64) as i64 + min_v as i64) as i32
    }

    pub fn next_float(&mut self, min_v: f32, max_v: f32) -> f32 {
        (self.next_double() as f32) * (max_v - min_v) + min_v
    }

    pub fn random_rate(&mut self, rate: f32) -> bool { 
        (self.next_double() as f32) < rate 
    }

    pub fn advance(&mut self, steps: usize) {
        for _ in 0..steps { self.next_double(); }
    }

    // 🌟新增：吸星大法级指针耗空源 - Box-Muller 正态极坐标转换
    pub fn box_muller_trap(&mut self) -> f32 {
        loop {
            let n1 = self.next_double() as f32 * 2.0 - 1.0;
            let n2 = self.next_double() as f32 * 2.0 - 1.0;
            let n3 = n1 * n1 + n2 * n2;
            if n3 > 0.0 && n3 < 1.0 { 
                return n1 * ((-2.0 * n3.ln()) / n3).sqrt(); 
            }
        }
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