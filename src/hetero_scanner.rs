use rayon::prelude::*;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use crate::rng::{GMathUtl, RandomType};

const T_SOIL: usize = 1;
const T_DEPTH_WATER: usize = 2;
const T_D_DEPTH_WATER: usize = 3;
const T_SHALLOW_WATER: usize = 4;
const T_MUD: usize = 5;
const T_FERTILE_SOIL: usize = 6;
const T_LING_SOIL: usize = 7;
const T_STONE_LAND: usize = 9;
const T_ROCK_BROWN: usize = 10;
const T_ROCK_GRAY: usize = 11;
const T_ROCK_MARBLE: usize = 12;
const T_IRON_ORE: usize = 13;
const T_COPPER_ORE: usize = 14;
const T_SILVER_ORE: usize = 15;
const T_BORN_SPACE: usize = 16;
const T_BORN_LINE: usize = 17;
const T_TMP_1: usize = 18;

#[derive(Clone, Copy, PartialEq)]
enum CType { AllTrue, NoBorn, CheckCon, CheckCon2 }

static BASE_AROUND_50: OnceLock<Vec<(i32, i32)>> = OnceLock::new();

fn get_base_around_50() -> &'static Vec<(i32, i32)> {
    BASE_AROUND_50.get_or_init(|| {
        let mut list = Vec::with_capacity(10201);
        list.push((0, 0));
        let mut num = 1; let mut i = 2; let mut dir = 1; let mut x = 0; let mut y = 0;
        let mut j = 1;
        while j < 10201 {
            let mut current_i = i;
            while current_i > 0 {
                for _ in 0..num {
                    match dir { 0 => x += 1, 1 => y -= 1, 2 => x -= 1, 3 => y += 1, _ => {} }
                    list.push((x, y)); j += 1;
                    if j >= 10201 { return list; }
                }
                dir = (dir + 1) % 4; current_i -= 1;
            }
            num += 1; i = 2;
        }
        list
    })
}

struct ShadowPipeline {
    bb: [[u32; 1152]; 20],
    rand: GMathUtl,
    mine_dirs: Vec<u32>,
}

impl ShadowPipeline {
    fn new(seed: i32) -> Self {
        Self { bb: [[0u32; 1152]; 20], rand: GMathUtl::new(seed), mine_dirs: Vec::with_capacity(2048) }
    }
    
    #[inline(always)]
    fn set_bit(&mut self, t_target: usize, word: usize, bit: u32) {
        for t in 1..=15 {
            if t == t_target { self.bb[t][word] |= bit; } else { self.bb[t][word] &= !bit; }
        }
    }
    
    #[inline(always)]
    fn apply_readback_word(&mut self, target: usize, w: usize, newly_added: u32) {
        for t in 1..=15 {
            if t == target {
                self.bb[t][w] |= newly_added;
            } else {
                self.bb[t][w] &= !newly_added;
            }
        }
    }
    
    fn get_ctype_mask(&self, i: usize, ctype: CType) -> u32 {
        match ctype {
            CType::AllTrue => 0xFFFFFFFF,
            CType::NoBorn => { let fs = self.bb[T_SOIL][i] | self.bb[T_FERTILE_SOIL][i]; let bp = self.bb[T_BORN_SPACE][i] | self.bb[T_BORN_LINE][i]; fs & !bp },
            CType::CheckCon => { self.bb[T_SOIL][i] | self.bb[T_FERTILE_SOIL][i] | self.bb[T_LING_SOIL][i] },
            CType::CheckCon2 => {
                let bad = self.bb[T_IRON_ORE][i] | self.bb[T_COPPER_ORE][i] | self.bb[T_SILVER_ORE][i] | self.bb[T_ROCK_BROWN][i] | self.bb[T_ROCK_GRAY][i];
                let marble = self.bb[T_ROCK_MARBLE][i]; let bp = self.bb[T_BORN_SPACE][i] | self.bb[T_BORN_LINE][i];
                !(bad | (marble & !bp))
            }
        }
    }
    
    fn get_cpu_neighbor(key: i32, out_keys: &mut [u32; 8]) -> usize {
        let mut count = 0;
        let dirs = [6, 4, 7, 5, 1, 2, 3, 0];
        for &dir in &dirs {
            let mut n = -1;
            if dir == 0 { n = key + 192; if n <= 0 || n >= 36864 { n = -1; } } 
            else if dir == 1 { n = key - 192; if n <= 0 || n >= 36864 { n = -1; } }
            else if dir == 2 { n = key - 1; if n < 0 || n >= 36864 || (key/192) != (n/192) { n = -1; } } 
            else if dir == 3 { n = key + 1; if n < 0 || n >= 36864 || (key/192) != (n/192) { n = -1; } }
            else if dir == 4 { let t = key - 1; if t >= 0 && t < 36864 && (key/192) == (t/192) { n = t - 192; if n <= 0 || n >= 36864 { n = -1; } } }
            else if dir == 5 { let t = key + 1; if t >= 0 && t < 36864 && (key/192) == (t/192) { n = t - 192; if n <= 0 || n >= 36864 { n = -1; } } }
            else if dir == 6 { let t = key - 1; if t >= 0 && t < 36864 && (key/192) == (t/192) { n = t + 192; if n <= 0 || n >= 36864 { n = -1; } } }
            else if dir == 7 { let t = key + 1; if t >= 0 && t < 36864 && (key/192) == (t/192) { n = t + 192; if n <= 0 || n >= 36864 { n = -1; } } }
            
            if n != -1 { out_keys[count] = n as u32; count += 1; }
        } count
    }
    
    fn step_cpu_random_fills(&mut self, target_t: usize, rcount: i32, ctype: CType) {
        let rc = rcount.max(1);
        for _ in 0..rc {
            let x = self.rand.random_range_int(0, 192, RandomType::EmNone, "MakeMap") as usize;
            let y = self.rand.random_range_int(0, 192, RandomType::EmNone, "MakeMap") as usize;
            let i = y * 6 + x / 32;
            let bit = 1 << (x % 32);
            if (self.get_ctype_mask(i, ctype) & bit) != 0 { self.set_bit(target_t, i, bit); }
        }
    }
    
    fn cpu_out_line(&mut self, src: usize, t_target: usize, w: i32, lv: i32, maxcount: i32, ctype: CType) {
        self.bb[T_TMP_1] = self.bb[src];
        let limit = ((w * 2 + 1) * (w * 2 + 1)) as usize; let base_around_50 = get_base_around_50();
        let mut current_max = maxcount;
        for word in (0..1152).rev() {
            let mut mask = self.bb[T_TMP_1][word];
            if word == 0 { mask &= !1; }
            while mask != 0 {
                let bit = 31 - mask.leading_zeros();
                mask ^= 1 << bit;
                let key = word * 32 + bit as usize;
                let cx = (key % 192) as i32; let cy = (key / 192) as i32;
                for i in 0..limit {
                    if i >= base_around_50.len() { break; }
                    let (dx, dy) = base_around_50[i];
                    let nx = cx + dx; let ny = cy + dy;
                    if nx >= 0 && nx < 192 && ny >= 0 && ny < 192 {
                        if self.rand.random_range_int(0, 100, RandomType::EmNone, "OutLine") > lv { continue; }
                        let nw = (ny * 192 + nx) as usize / 32;
                        let n_bit = 1 << ((ny * 192 + nx) as usize % 32);
                        if (self.get_ctype_mask(nw, ctype) & n_bit) != 0 {
                            self.set_bit(t_target, nw, n_bit);
                            current_max -= 1; if current_max == 0 { break; }
                        }
                    }
                }
            }
        }
    }
    
    fn cpu_random_line_from_mine_dir(&mut self, w: i32, size: i32, def: usize, ctype: CType) {
        if self.mine_dirs.is_empty() { return; }
        let mut num = self.rand.random_range_int(0, self.mine_dirs.len() as i32, RandomType::EmNone, "MakeMap") as usize;
        for _ in 0..size {
            if num >= self.mine_dirs.len() { num = self.rand.random_range_int(0, self.mine_dirs.len() as i32, RandomType::EmNone, "MakeMap") as usize; }
            let key = self.mine_dirs[num]; num += 1;
            let mut v_keys = [0u32; 8]; let n_count = Self::get_cpu_neighbor(key as i32, &mut v_keys);
            let mut f_key = key;
            if n_count > 0 { f_key = v_keys[self.rand.random_range_int(0, n_count as i32, RandomType::EmNone, "MakeMap") as usize]; }
            let nw = (f_key / 32) as usize;
            let nbit = 1 << (f_key % 32);
            if (self.get_ctype_mask(nw, ctype) & nbit) != 0 { self.set_bit(def, nw, nbit); }
        }
        self.cpu_out_line(def, def, w, 4, 0, ctype);
    }
    
    fn cpu_random_and_expand(&mut self, def: usize, rcount: i32, ecount: i32, expand_lv: i32, ctype: CType, ectype: CType) {
        self.step_cpu_random_fills(def, rcount, ctype);
        let mut flag = true;
        for _ in 0..ecount {
            if flag {
                for w in 0..1152 {
                    let mut b = 0;
                    while b < 32 {
                        let mask = self.bb[def][w] >> b;
                        if mask == 0 { break; } b += mask.trailing_zeros(); 
                        let key = w * 32 + b as usize;
                        if self.rand.random_range_int(0, 100, RandomType::EmNone, "MakeMap") <= expand_lv {
                            let mut n_keys = [0u32; 8]; let n_count = Self::get_cpu_neighbor(key as i32, &mut n_keys);
                            for d in 0..n_count {
                                let nk = n_keys[d] as usize;
                                let nw = nk / 32; let n_bit = 1 << (nk % 32);
                                if (self.get_ctype_mask(nw, ectype) & n_bit) != 0 { self.set_bit(def, nw, n_bit); }
                            }
                        } b += 1;
                    }
                } flag = false;
            } else {
                for w in (0..1152).rev() {
                    let mut b = 31i32;
                    while b >= 0 {
                        let bit_mask = if b == 31 { 0xFFFFFFFF } else { (1u32 << (b + 1)) - 1 };
                        let mask = self.bb[def][w] & bit_mask; if mask == 0 { break; }
                        b = 31 - mask.leading_zeros() as i32;
                        let key = w * 32 + b as usize;
                        if self.rand.random_range_int(0, 100, RandomType::EmNone, "MakeMap") <= expand_lv {
                            let mut n_keys = [0u32; 8]; let n_count = Self::get_cpu_neighbor(key as i32, &mut n_keys);
                            for d in 0..n_count {
                                let nk = n_keys[d] as usize;
                                let nw = nk / 32; let n_bit = 1 << (nk % 32);
                                if (self.get_ctype_mask(nw, ectype) & n_bit) != 0 { self.set_bit(def, nw, n_bit); }
                            }
                        } b -= 1;
                    }
                } flag = true;
            }
        }
    }
    
    fn step_1_cpu(&mut self) {
        for i in 0..1152 { self.bb[T_SOIL][i] = 0xFFFFFFFF; }
        for _ in 0..3 {
            let mut i = 0;
            let mut num = self.rand.random_range_int(0, 192, RandomType::EmNone, "MakeMineDir");
            let num2 = num; let _ = self.rand.random_range_int(0, 192, RandomType::EmNone, "MakeMineDir");
            while i < 192 {
                if self.rand.random_range_int(0, 100, RandomType::EmNone, "MakeMineDir") < 10 { i -= 1; } else { i += 1; }
                if num2 > 96 { num += self.rand.random_range_int(-3, 2, RandomType::EmNone, "MakeMineDir"); } else { num += self.rand.random_range_int(-2, 3, RandomType::EmNone, "MakeMineDir"); }
                if i >= 0 && i < 192 && num >= 0 && num < 192 { let key = num * 192 + i; if key > 0 { self.mine_dirs.push(key as u32); } }
            }
        }
        let kx = self.rand.random_range_int(76, 133, RandomType::EmNone, "MakeMap");
        let ky = self.rand.random_range_int(114, 133, RandomType::EmNone, "MakeMap");
        if kx < 192 && ky < 192 { self.set_bit(T_FERTILE_SOIL, (ky * 6 + kx / 32) as usize, 1 << (kx % 32)); }
    }
    
    fn step_2_cpu_a(&mut self) {
        for i in 0..1152 { let mut m = self.bb[T_FERTILE_SOIL][i]; if i == 0 { m &= !1; } self.bb[T_BORN_SPACE][i] = m; }
    }
    
    fn step_2_cpu_b(&mut self) {
        for i in 0..1152 { let mut m = self.bb[T_FERTILE_SOIL][i]; if i == 0 { m &= !1; } self.set_bit(T_SOIL, i, m); }
        for i in 0..4 {
            let num2 = self.rand.random_range_int(57, 115, RandomType::EmNone, "MakeMap");
            let mut j = 0;
            while j < self.rand.random_range_int(5, 15, RandomType::EmNone, "MakeMap") {
                let mut k = -1;
                let ny_nx = num2 + j;
                if ny_nx < 192 { k = if i == 0 { ny_nx * 192 } else if i == 1 { ny_nx * 192 + 191 } else if i == 2 { ny_nx } else { 191 * 192 + ny_nx }; }
                if k >= 0 && k < 36864 { self.bb[T_BORN_LINE][(k / 32) as usize] |= 1 << (k % 32); }
                j += 1;
            }
        }
    }
    
    fn step_6_pre_rock_opt(&mut self) {
        let ss = 3;
        let mut i1 = 0; 
        while i1 < self.rand.random_range_int(ss, ss + 2, RandomType::EmNone, "MakeMap") { 
            let arg1 = self.rand.random_range_int(0, ss, RandomType::EmNone, "MakeMap");
            let arg2 = self.rand.random_range_int(5 + ss, 10 + ss, RandomType::EmNone, "MakeMap");
            self.cpu_random_line_from_mine_dir(arg1, arg2, T_IRON_ORE, CType::NoBorn); 
            i1 += 1;
        }
        
        let mut i2 = 0;
        while i2 < 1 + ss { 
            let arg1 = self.rand.random_range_int(0, 1, RandomType::EmNone, "MakeMap");
            let arg2 = self.rand.random_range_int(3 + ss, 5 + ss, RandomType::EmNone, "MakeMap");
            self.cpu_random_line_from_mine_dir(arg1, arg2, T_COPPER_ORE, CType::NoBorn); 
            i2 += 1;
        }
        
        let mut i3 = 0;
        while i3 < 1 + ss { 
            let arg1 = self.rand.random_range_int(0, 1, RandomType::EmNone, "MakeMap");
            let arg2 = self.rand.random_range_int(3 + ss, 5 + ss, RandomType::EmNone, "MakeMap");
            self.cpu_random_line_from_mine_dir(arg1, arg2, T_SILVER_ORE, CType::NoBorn); 
            i3 += 1;
        }
        
        let mut i4 = 0;
        while i4 < self.rand.random_range_int(1, 3, RandomType::EmNone, "MakeMap") { 
            let w = self.rand.random_range_int(0, 1, RandomType::EmNone, "MakeMap");
            let s = self.rand.random_range_int(8, 16, RandomType::EmNone, "MakeMap"); 
            let st = if self.rand.random_range_int(1, 3, RandomType::EmNone, "MakeMap") == 1 { T_ROCK_GRAY } else { T_ROCK_MARBLE };
            self.cpu_random_line_from_mine_dir(w, s, st, CType::NoBorn); 
            i4 += 1; 
        }
    }
    
    fn step_7_cpu(&mut self) {
        self.cpu_out_line(T_ROCK_BROWN, T_STONE_LAND, 1, 30, 0, CType::CheckCon2);
        self.cpu_out_line(T_IRON_ORE, T_STONE_LAND, 1, 30, 0, CType::CheckCon2); self.cpu_out_line(T_SILVER_ORE, T_STONE_LAND, 1, 30, 0, CType::CheckCon2); self.cpu_out_line(T_COPPER_ORE, T_STONE_LAND, 1, 30, 0, CType::CheckCon2);
        self.cpu_out_line(T_ROCK_BROWN, T_STONE_LAND, 1, 30, 0, CType::CheckCon2);
        self.cpu_out_line(T_STONE_LAND, T_STONE_LAND, 1, 5, 0, CType::CheckCon);
    }
}

const WGSL_HETERO_SHADER: &str = r#"
struct Config { batch_size: u32, opt_min: u32, opt_max: u32, pad: u32 }
@group(0) @binding(0) var<uniform> cfg: Config;
@group(0) @binding(1) var<storage, read_write> buf_a: array<u32>;
@group(0) @binding(2) var<storage, read> buf_b: array<u32>;
@group(0) @binding(3) var<storage, read_write> buf_out: array<u32>;

@compute @workgroup_size(64)
fn optimize_pass(@builtin(global_invocation_id) id: vec3<u32>) {
    let task = id.x; if (task >= cfg.batch_size) { return; }
    let offset = task * 1152u;
    
    var pr = array<u32, 6>(0u,0u,0u,0u,0u,0u);
    for (var y = 0u; y < 192u; y++) {
        let b = offset + y * 6u;
        var cr = array<u32, 6>(buf_a[b], buf_a[b+1u], buf_a[b+2u], buf_a[b+3u], buf_a[b+4u], buf_a[b+5u]);
        var nr = array<u32, 6>(0u,0u,0u,0u,0u,0u);
        if (y < 191u) { let nb = offset + (y + 1u) * 6u; nr = array<u32, 6>(buf_a[nb], buf_a[nb+1u], buf_a[nb+2u], buf_a[nb+3u], buf_a[nb+4u], buf_a[nb+5u]); }
        for (var xw = 0u; xw < 6u; xw++) {
            let w = b + xw;
            let mask = buf_b[w]; var f_mask = 0u;
            if (mask != 0u) {
                let c = cr[xw];
                var cl = c >> 1u; if (xw < 5u) { cl |= (cr[xw+1u] << 31u); } 
                var cr_b = c << 1u; if (xw > 0u) { cr_b |= (cr[xw-1u] >> 31u); }
                var u = pr[xw];
                if (y == 1u && xw == 0u) { u &= 0xFFFFFFFEu; }
                var ul = u >> 1u;
                if (xw < 5u) { ul |= (pr[xw+1u] << 31u); } var ur = u << 1u;
                if (xw > 0u) { ur |= (pr[xw-1u] >> 31u); }
                var d = nr[xw];
                var dl = d >> 1u; if (xw < 5u) { dl |= (nr[xw+1u] << 31u); } 
                var dr = d << 1u; if (xw > 0u) { dr |= (nr[xw-1u] >> 31u); }
                for (var bit = 0u; bit < 32u; bit++) {
                    let bm = 1u << bit;
                    if ((mask & bm) != 0u) {
                        var count = 0u;
                        if ((cl & bm) != 0u) { count++; } if ((cr_b & bm) != 0u) { count++; }
                        if ((u & bm) != 0u) { count++; } if ((ul & bm) != 0u) { count++; } if ((ur & bm) != 0u) { count++; }
                        if ((d & bm) != 0u) { count++; } if ((dl & bm) != 0u) { count++; } if ((dr & bm) != 0u) { count++; }
                        
                        var is_valid = false;
                        if (count >= cfg.opt_min && count <= cfg.opt_max) { 
                            is_valid = true;
                        }
                        if (is_valid) { f_mask |= bm; }
                    }
                }
            } buf_out[w] = f_mask;
        } pr = cr;
    }
}

@compute @workgroup_size(64)
fn apply_tmp(@builtin(global_invocation_id) id: vec3<u32>) {
    let task = id.x;
    if (task >= cfg.batch_size) { return; }
    let offset = task * 1152u;
    for (var i = 0u; i < 1152u; i++) { buf_a[offset + i] |= buf_out[offset + i]; }
}

@compute @workgroup_size(64)
fn count_bits(@builtin(global_invocation_id) id: vec3<u32>) {
    let task = id.x; if (task >= cfg.batch_size) { return; }
    let offset = task * 1152u; var count = 0u;
    for (var i = 0u; i < 1152u; i++) { count += countOneBits(buf_a[offset + i]); }
    buf_out[task] = count;
}
"#;

struct GpuBuffers {
    cfg_buf: wgpu::Buffer,
    gpu_layer: wgpu::Buffer,
    gpu_valid: wgpu::Buffer,
    gpu_tmp: wgpu::Buffer,
    readback_buf: wgpu::Buffer,
    bg_opt: wgpu::BindGroup,
    bg_cnt: wgpu::BindGroup,
}

pub fn scan_seeds_heterogeneous(start: i32, end: i32, _map_size: i32, threshold: usize, progress: Arc<AtomicUsize>) -> Vec<(i32, usize)> {
    let seed_counter = Arc::new(AtomicI64::new(start as i64));
    
    let pure_gpu_thread = {
        let seed_counter = seed_counter.clone();
        let progress = progress.clone();
        std::thread::spawn(move || {
            crate::gpu_scanner::run_pure_gpu_dynamic(seed_counter, end, threshold, progress)
        })
    };
    
    let mut hetero_results = pollster::block_on(run_dma_pipeline(seed_counter, end, threshold, progress));
    
    let mut pure_results = pure_gpu_thread.join().unwrap();
    
    hetero_results.append(&mut pure_results);
    hetero_results.sort_by(|a, b| b.1.cmp(&a.1));
    hetero_results
}

async fn run_dma_pipeline(seed_counter: Arc<AtomicI64>, end: i32, threshold: usize, progress: Arc<AtomicUsize>) -> Vec<(i32, usize)> {
    let initial_start = seed_counter.load(Ordering::Relaxed);
    let total = (end as i64 - initial_start + 1).max(0) as u32; 
    if total == 0 { return vec![]; }
    
    let instance = wgpu::Instance::default();
    let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }).await.unwrap();
    let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.unwrap();
    let device = Arc::new(device);
    let queue = Arc::new(queue);
    
    let poll_device = device.clone();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        while stop_rx.try_recv().is_err() {
            let _ = poll_device.poll(wgpu::PollType::Poll);
            std::thread::sleep(std::time::Duration::from_micros(500));
        }
    });
    
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(WGSL_HETERO_SHADER.into()) });
    
    let layout = Arc::new(device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[
        wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
        wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
        wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
        wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
    ]}));
    
    let pipe_opt = Arc::new(device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&*layout)], ..Default::default() })), module: &shader, entry_point: Some("optimize_pass"), cache: None, compilation_options: Default::default() }));
    let pipe_apply = Arc::new(device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&*layout)], ..Default::default() })), module: &shader, entry_point: Some("apply_tmp"), cache: None, compilation_options: Default::default() }));
    let pipe_cnt = Arc::new(device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&*layout)], ..Default::default() })), module: &shader, entry_point: Some("count_bits"), cache: None, compilation_options: Default::default() }));
    
    let batch = 500usize; 
    let bb_size = (batch * 1152 * 4) as u64;
    let num_cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(16);
    let pool_size = num_cores + 2;

    let (buf_tx, buf_rx) = std::sync::mpsc::channel();
    for _ in 0..pool_size { 
        let cfg_buf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let gpu_layer = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bb_size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let gpu_valid = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bb_size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let gpu_tmp = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bb_size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let readback_buf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bb_size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        
        let bg_opt = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &[ 
            wgpu::BindGroupEntry { binding: 0, resource: cfg_buf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: gpu_layer.as_entire_binding() }, 
            wgpu::BindGroupEntry { binding: 2, resource: gpu_valid.as_entire_binding() }, wgpu::BindGroupEntry { binding: 3, resource: gpu_tmp.as_entire_binding() }
        ]});
        let bg_cnt = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &[ 
            wgpu::BindGroupEntry { binding: 0, resource: cfg_buf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: gpu_layer.as_entire_binding() }, 
            wgpu::BindGroupEntry { binding: 2, resource: gpu_valid.as_entire_binding() }, wgpu::BindGroupEntry { binding: 3, resource: gpu_tmp.as_entire_binding() }
        ]});
        buf_tx.send(GpuBuffers { cfg_buf, gpu_layer, gpu_valid, gpu_tmp, readback_buf, bg_opt, bg_cnt }).unwrap();
    }
    
    let buf_rx = std::sync::Mutex::new(buf_rx);
    
    let iter = std::iter::from_fn(|| {
        let s = seed_counter.fetch_add(batch as i64, Ordering::Relaxed);
        if s <= end as i64 { Some(s) } else { None }
    });

    let mut all_results: Vec<(i32, usize)> = iter.par_bridge().flat_map(|current_start| {
        let diff = ((end as i64 - current_start + 1) as usize).min(batch);
        let mut engines: Vec<_> = (0..diff).map(|i| ShadowPipeline::new((current_start + i as i64) as i32)).collect();
        
        let buffers = buf_rx.lock().unwrap().recv().unwrap();
        
        let mut b_layer = vec![0u32; diff * 1152];
        let mut b_valid = vec![0u32; diff * 1152];
        
        let run_gpu_pass = |cmd: &str, radius: u32, opt_min: u32, opt_max: u32, read_back: bool| -> Option<Vec<u32>> {
            queue.write_buffer(&buffers.cfg_buf, 0, bytemuck::cast_slice(&[diff as u32, opt_min, opt_max, radius]));
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            
            if cmd == "optimize" { 
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                    pass.set_pipeline(&pipe_opt); 
                    pass.set_bind_group(0, &buffers.bg_opt, &[]); 
                    pass.dispatch_workgroups(((diff + 63)/64) as u32, 1, 1);
                }
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                    pass.set_pipeline(&pipe_apply); 
                    pass.set_bind_group(0, &buffers.bg_opt, &[]); 
                    pass.dispatch_workgroups(((diff + 63)/64) as u32, 1, 1);
                }
            } 
            else if cmd == "count" { 
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                pass.set_pipeline(&pipe_cnt); 
                pass.set_bind_group(0, &buffers.bg_cnt, &[]); 
                pass.dispatch_workgroups(((diff + 63)/64) as u32, 1, 1);
            }
            
            if read_back {
                let read_src = if cmd == "count" { &buffers.gpu_tmp } else { &buffers.gpu_layer };
                let copy_size = if cmd == "count" { (diff * 4) as u64 } else { diff as u64 * 1152 * 4 };
                
                encoder.copy_buffer_to_buffer(read_src, 0, &buffers.readback_buf, 0, copy_size); 
                queue.submit(Some(encoder.finish()));
                
                let slice = buffers.readback_buf.slice(0..copy_size); 
                let (tx, rx) = std::sync::mpsc::channel();
                slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
                
                while rx.try_recv().is_err() { std::thread::yield_now(); }
                
                let data = slice.get_mapped_range();
                let out_data: &[u32] = bytemuck::cast_slice(&data); 
                let res = out_data.to_vec();
                
                drop(data); 
                buffers.readback_buf.unmap();
                Some(res)
            } else { 
                queue.submit(Some(encoder.finish()));
                None 
            }
        };
        
        let mut run_opt = |engines: &mut Vec<ShadowPipeline>, target_t: usize, opt_min: u32, opt_max: u32, maxcount: usize, ctype: CType| {
            for _ in 0..maxcount {
                for (i, e) in engines.iter().enumerate() { 
                    b_layer[i*1152..(i+1)*1152].copy_from_slice(&e.bb[target_t]);
                    for w_idx in 0..1152 { b_valid[i*1152 + w_idx] = e.get_ctype_mask(w_idx, ctype); } 
                }
                queue.write_buffer(&buffers.gpu_layer, 0, bytemuck::cast_slice(&b_layer));
                queue.write_buffer(&buffers.gpu_valid, 0, bytemuck::cast_slice(&b_valid));
                
                let optimized = run_gpu_pass("optimize", 0, opt_min, opt_max, true).unwrap();
                for (i, e) in engines.iter_mut().enumerate() { 
                    let offset = i * 1152;
                    for w_idx in 0..1152 { 
                        let newly_added = optimized[offset + w_idx] & !e.bb[target_t][w_idx];
                        if newly_added != 0 { e.apply_readback_word(target_t, w_idx, newly_added); } 
                    } 
                }
            }
        };

        for e in engines.iter_mut() { e.step_1_cpu(); }
        
        for e in engines.iter_mut() {
            e.cpu_out_line(T_FERTILE_SOIL, T_FERTILE_SOIL, 2, 100, 0, CType::AllTrue);
            e.cpu_out_line(T_FERTILE_SOIL, T_FERTILE_SOIL, 5, 20, 0, CType::AllTrue);
        }
        
        run_opt(&mut engines, T_FERTILE_SOIL, 2, 4, 1, CType::AllTrue);
        
        for e in engines.iter_mut() { e.step_2_cpu_a(); }
        for e in engines.iter_mut() { e.cpu_out_line(T_FERTILE_SOIL, T_FERTILE_SOIL, 2, 100, 0, CType::AllTrue); }
        for e in engines.iter_mut() {
            e.step_2_cpu_b();
            e.cpu_random_and_expand(T_FERTILE_SOIL, 20, 4, 30, CType::AllTrue, CType::AllTrue);
        }

        let ss = 3;
        for e in engines.iter_mut() {
            e.cpu_random_and_expand(T_D_DEPTH_WATER, ss as i32 - 1, 2 * ss as i32 - 1, 13 + 6 * ss as i32, CType::NoBorn, CType::NoBorn);
        }

        for e in engines.iter_mut() {
            e.cpu_out_line(T_D_DEPTH_WATER, T_DEPTH_WATER, 1, 100, 0, CType::CheckCon);
            e.cpu_out_line(T_DEPTH_WATER, T_DEPTH_WATER, 1, 10 + 6 * ss as i32, 0, CType::NoBorn);
            e.cpu_out_line(T_DEPTH_WATER, T_SHALLOW_WATER, 4, 50 + 12 * ss as i32, 0, CType::CheckCon);
        }
        run_opt(&mut engines, T_SHALLOW_WATER, 2, 6, 1, CType::CheckCon);

        for e in engines.iter_mut() {
            e.cpu_random_and_expand(T_SHALLOW_WATER, 3, 3, 20, CType::NoBorn, CType::NoBorn);
        }

        for e in engines.iter_mut() {
            e.cpu_out_line(T_SHALLOW_WATER, T_MUD, 4, 90, 0, CType::CheckCon);
            e.step_6_pre_rock_opt();
        }

        for e in engines.iter_mut() {
            let mut m = 1;
            while m < e.rand.random_range_int(1, 3, RandomType::EmNone, "MakeMap") { 
                let st = if m == 1 { T_ROCK_GRAY } else { T_ROCK_MARBLE };
                let random_count = e.rand.random_range_int(0, 3, RandomType::EmNone, "MakeMap");
                e.cpu_random_and_expand(st, random_count, 3, 15 + 3 * ss as i32, CType::NoBorn, CType::NoBorn);
                m += 1; 
            }
            
            let iron_rand_1 = e.rand.random_range_int(0, ss as i32 + 1, RandomType::EmNone, "MakeMap");
            let iron_rand_2 = e.rand.random_range_int(1, ss as i32 + 1, RandomType::EmNone, "MakeMap");
            let iron_rand_3 = e.rand.random_range_int(1, 4, RandomType::EmNone, "MakeMap");
            e.cpu_random_and_expand(T_IRON_ORE, iron_rand_1, iron_rand_2, 13 + ss as i32 * iron_rand_3, CType::NoBorn, CType::NoBorn);

            let cu_rand_1 = e.rand.random_range_int(0, ss as i32 + 1, RandomType::EmNone, "MakeMap");
            let cu_rand_2 = e.rand.random_range_int(1, ss as i32 + 1, RandomType::EmNone, "MakeMap");
            let cu_rand_3 = e.rand.random_range_int(1, 4, RandomType::EmNone, "MakeMap");
            e.cpu_random_and_expand(T_COPPER_ORE, cu_rand_1, cu_rand_2, 13 + ss as i32 * cu_rand_3, CType::NoBorn, CType::NoBorn);

            let ag_rand_1 = e.rand.random_range_int(0, ss as i32 + 1, RandomType::EmNone, "MakeMap");
            let ag_rand_2 = e.rand.random_range_int(1, ss as i32 + 1, RandomType::EmNone, "MakeMap");
            let ag_rand_3 = e.rand.random_range_int(1, 4, RandomType::EmNone, "MakeMap");
            e.cpu_random_and_expand(T_SILVER_ORE, ag_rand_1, ag_rand_2, 13 + ss as i32 * ag_rand_3, CType::NoBorn, CType::NoBorn);

            let rb_rand_1 = e.rand.random_range_int(2, ss as i32 + 1, RandomType::EmNone, "MakeMap");
            e.cpu_random_and_expand(T_ROCK_BROWN, rb_rand_1, ss as i32 + 1, 13 + ss as i32 * 4, CType::NoBorn, CType::NoBorn);

            e.cpu_out_line(T_ROCK_GRAY, T_ROCK_BROWN, 1, 50 + 12 * ss as i32, 0, CType::NoBorn);
            e.cpu_out_line(T_ROCK_MARBLE, T_ROCK_BROWN, 1, 50 + 12 * ss as i32, 0, CType::NoBorn);
            e.cpu_out_line(T_IRON_ORE, T_ROCK_BROWN, 1, 50 + 12 * ss as i32, 0, CType::NoBorn);
            e.cpu_out_line(T_COPPER_ORE, T_ROCK_BROWN, 1, 50 + 12 * ss as i32, 0, CType::NoBorn);
            e.cpu_out_line(T_SILVER_ORE, T_ROCK_BROWN, 1, 50 + 12 * ss as i32, 0, CType::NoBorn);
            
            let arg_iron = e.rand.random_range_int(1, 2, RandomType::EmNone, "MakeMap");
            e.cpu_out_line(T_IRON_ORE, T_ROCK_BROWN, arg_iron, 8 + 8 * ss as i32, 0, CType::NoBorn);
            
            let arg_cu = e.rand.random_range_int(1, 2, RandomType::EmNone, "MakeMap");
            e.cpu_out_line(T_COPPER_ORE, T_ROCK_BROWN, arg_cu, 8 + 8 * ss as i32, 0, CType::NoBorn);
            
            let arg_ag = e.rand.random_range_int(1, 2, RandomType::EmNone, "MakeMap");
            e.cpu_out_line(T_SILVER_ORE, T_ROCK_BROWN, arg_ag, 8 + 8 * ss as i32, 0, CType::NoBorn);

            for j in 0..3 { 
                let t = if j == 1 { T_ROCK_GRAY } else if j == 2 { T_ROCK_MARBLE } else { T_ROCK_BROWN };
                let arg_r = e.rand.random_range_int(1, ss as i32, RandomType::EmNone, "MakeMap");
                e.cpu_out_line(t, T_ROCK_BROWN, arg_r, 8 + 8 * ss as i32, 0, CType::NoBorn);
            }
        }
        
        run_opt(&mut engines, T_ROCK_BROWN, 2, 9, 1, CType::NoBorn);

        for e in engines.iter_mut() { e.step_7_cpu(); }
        run_opt(&mut engines, T_STONE_LAND, 2, 9, 1, CType::CheckCon);

        for e in engines.iter_mut() { e.cpu_random_and_expand(T_LING_SOIL, 3, 6, 33, CType::CheckCon, CType::CheckCon); }

        for (i, e) in engines.iter().enumerate() { b_layer[i*1152..(i+1)*1152].copy_from_slice(&e.bb[T_LING_SOIL]); }
        queue.write_buffer(&buffers.gpu_layer, 0, bytemuck::cast_slice(&b_layer));
        
        let scores = run_gpu_pass("count", 0, 0, 0, true).unwrap();
        
        let mut batch_results = Vec::new();
        for i in 0..diff {
            if scores[i] as usize >= threshold { batch_results.push(((current_start + i as i64) as i32, scores[i] as usize)); }
        }
        
        progress.fetch_add(diff, Ordering::Relaxed);
        buf_tx.send(buffers).unwrap();
        batch_results
    }).collect();
    
    let _ = stop_tx.send(());
    all_results
}