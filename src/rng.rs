use std::fs::File;
use std::io::{Write, BufWriter};
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum RandomType {
    EmNone = 0,
}
pub struct DotNetRandom {
    inext: usize,
    inextp: usize,
    seed_array: [i32; 56],
}
impl DotNetRandom {
    pub fn new(seed: i32) -> Self {        
        let mut rng = DotNetRandom { inext: 0, inextp: 31, seed_array: [0; 56] };
        
        let subtraction = if seed == i32::MIN { i32::MAX } else { seed.abs() };
        let mut mj = 161803398 - subtraction;
        rng.seed_array[55] = mj;
        let mut mk = 1;
        for i in 1..55 {
            let ii = (21 * i) % 55;
            rng.seed_array[ii] = mk;
            mk = mj - mk;
            if mk < 0 { mk += 2147483647; }
            mj = rng.seed_array[ii];
        }
        for _ in 1..5 {
            for k in 1..56 {
                rng.seed_array[k] -= rng.seed_array[1 + (k + 30) % 55];
                if rng.seed_array[k] < 0 { rng.seed_array[k] += 2147483647; }
            }
        }
        
        rng.inext = 0;        
        rng.inextp = 31; 
        rng
    }
    
    fn sample(&mut self) -> f64 {
        self.inext = if self.inext + 1 >= 56 { 1 } else { self.inext + 1 };
        self.inextp = if self.inextp + 1 >= 56 { 1 } else { self.inextp + 1 };
        let mut num = self.seed_array[self.inext] - self.seed_array[self.inextp];
        if num < 0 { num += 2147483647; }
        
        self.seed_array[self.inext] = num;
        (num as f64) * 4.656612875245797E-10
    }
    pub fn next_range(&mut self, mut min_value: i32, mut max_value: i32) -> i32 {
        if min_value > max_value { std::mem::swap(&mut min_value, &mut max_value); }
        
        let num_diff = (max_value as i64) - (min_value as i64);
        let num_uint = num_diff as u32;
        if num_uint <= 1 {
            return min_value;
        }
        let sample_val = self.sample();
        let scaled_uint = (sample_val * (num_uint as f64)) as u32;
        let result = (scaled_uint as u64).wrapping_add((min_value as i64) as u64) as i32;
        result
    }
}
pub struct GMathUtl {
    sys_random: DotNetRandom,
    pub is_logging: bool,
    pub call_count: usize,
    pub log_writer: Option<BufWriter<File>>,
}
impl GMathUtl {
    pub fn new(seed: i32) -> Self {
        Self { 
            sys_random: DotNetRandom::new(seed),
            is_logging: false,
            call_count: 0,
            log_writer: None,
        }
    }
    
    pub fn enable_logging(&mut self, filepath: &str) {
        if let Ok(file) = File::create(filepath) {
            let mut writer = BufWriter::new(file);
            writeln!(writer, "Order,Type,Caller,Min,Max,Result").unwrap();
            self.log_writer = Some(writer);
            self.is_logging = true;
        }
    }
    
    pub fn random_range_int(&mut self, min: i32, max: i32, _r_type: RandomType, caller: &str) -> i32 {
        let res = self.sys_random.next_range(min, max);
        if self.is_logging {
            self.call_count += 1;
            if let Some(writer) = &mut self.log_writer {
                writeln!(writer, "{},INT,{},{},{},{}", self.call_count, caller, min, max, res).unwrap();
            }
        }
        res
    }
}
pub fn string_hash(s: &str) -> i32 {
    let mut num: i32 = 0;
    for c in s.encode_utf16() { num = num.wrapping_shl(5).wrapping_sub(num).wrapping_add(c as i32); }
    num
}
pub fn find_chinese_collision(target_seed: i32) -> Option<String> {
    let target = target_seed as u32;
    
    let watermark = "玄黎曰"; 
    let mut watermark_hash = 0u32;
    for c in watermark.encode_utf16() {
        watermark_hash = watermark_hash.wrapping_mul(31).wrapping_add(c as u32);
    }
    let mut offset = watermark_hash;
    for _ in 0..5 {
        offset = offset.wrapping_mul(31);
    }
    let base_char = 0x7384u32; 
    let mut base_val = 0u32;
    for _ in 0..5 { 
        base_val = base_val.wrapping_mul(31).wrapping_add(base_char); 
    }
    
    let mut rem = target.wrapping_sub(offset).wrapping_sub(base_val);
    
    let mut d = [0u32; 5];
    d[0] = rem / 923521;  rem %= 923521;
    d[1] = rem / 29791;   rem %= 29791;
    d[2] = rem / 961;     rem %= 961;
    d[3] = rem / 31;      d[4] = rem % 31;
    
    let chars: [u16; 5] = [
        (base_char + d[0]) as u16, 
        (base_char + d[1]) as u16,
        (base_char + d[2]) as u16, 
        (base_char + d[3]) as u16, 
        (base_char + d[4]) as u16,
    ];
    
    if let Ok(suffix) = String::from_utf16(&chars) {
        let final_seed = format!("{}{}", watermark, suffix);
        Some(final_seed)
    } else {
        None
    }
}