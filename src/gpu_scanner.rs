use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;

pub fn scan_seeds_amd_gpu(
    start: i32,
    end: i32,
    _map_size: i32,
    threshold: usize,
    progress: Arc<AtomicUsize>,
) -> Vec<(i32, usize)> {
    let seed_counter = Arc::new(AtomicI64::new(start as i64));
    pollster::block_on(run_wgpu_compute_architecture_dynamic(seed_counter, end, threshold, progress))
}

pub fn run_pure_gpu_dynamic(
    current_seed: Arc<AtomicI64>,
    end: i32,
    threshold: usize,
    progress: Arc<AtomicUsize>,
) -> Vec<(i32, usize)> {
    pollster::block_on(run_wgpu_compute_architecture_dynamic(current_seed, end, threshold, progress))
}

async fn run_wgpu_compute_architecture_dynamic(
    current_seed: Arc<AtomicI64>,
    end: i32,
    threshold: usize,
    progress: Arc<AtomicUsize>,
) -> Vec<(i32, usize)> {
    let mut results_vec = Vec::new();
    
    if current_seed.load(Ordering::Relaxed) > end as i64 {
        return results_vec;
    }

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter_opt = instance.request_adapter(&wgpu::RequestAdapterOptions { 
        power_preference: wgpu::PowerPreference::HighPerformance, 
        ..Default::default() 
    }).await;
    
    let adapter = match adapter_opt {
        Ok(a) => a,
        Err(_) => return results_vec,
    };
    
    let actual_limits = adapter.limits();
    let safe_limits = actual_limits.clone();
    
    let (device, queue) = match adapter.request_device(&wgpu::DeviceDescriptor { 
        label: None, 
        required_features: wgpu::Features::empty(), 
        required_limits: safe_limits,
        ..Default::default()
    }).await {
        Ok(dq) => dq,
        Err(_) => return results_vec,
    };
    
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { 
        label: None, 
        source: wgpu::ShaderSource::Wgsl(WGSL_SHADER.into()) 
    });
    
    let bytes_per_task = 20 * 1152 * 4; 
    let diff = (end as i64 - current_seed.load(Ordering::Relaxed) + 1).max(0) as u32;
    let mut chunk_size = (actual_limits.max_storage_buffer_binding_size as u64 / bytes_per_task).min(30000).min(diff as u64) as u32;
    chunk_size = (chunk_size & !127).max(128);
    
    let grids_buffer = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (chunk_size as u64) * bytes_per_task, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
    let mine_dirs_buffer = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (chunk_size as u64) * 2048 * 4, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
    let mine_dir_counts_buffer = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (chunk_size as u64) * 4, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
    let result_buffer = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (chunk_size as u64) * 4, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
    let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (chunk_size as u64) * 4, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let config_buffer = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None, entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
        ]
    });
    
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None, layout: &layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: config_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: result_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: grids_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: mine_dirs_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: mine_dir_counts_buffer.as_entire_binding() },
        ]
    });
    
    let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { 
        label: None, 
        layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { 
            label: None, 
            bind_group_layouts: &[Some(&layout)], 
            ..Default::default() 
        })), 
        module: &shader, 
        entry_point: Some("main"),
        cache: None,
        compilation_options: Default::default(),
    });
    
    loop {
        let current_start = current_seed.fetch_add(chunk_size as i64, Ordering::Relaxed);
        if current_start > end as i64 {
            break;
        }
        
        let current_diff = ((end as i64 - current_start + 1) as u64).min(chunk_size as u64) as u32;
        queue.write_buffer(&config_buffer, 0, bytemuck::cast_slice(&[current_start as i32, current_diff as i32, chunk_size as i32, threshold as i32]));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
            cpass.set_pipeline(&compute_pipeline);
            cpass.set_bind_group(0, &bind_group, &[]);
            cpass.dispatch_workgroups((current_diff + 127) / 128, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&result_buffer, 0, &readback_buffer, 0, (current_diff as u64) * 4);
        queue.submit(Some(encoder.finish()));
        
        let slice = readback_buffer.slice(0..((current_diff as u64) * 4));
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
        
        device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        
        if let Ok(Ok(())) = rx.recv() {
            let data = slice.get_mapped_range();
            let counts: &[u32] = bytemuck::cast_slice(&data);
            for i in 0..current_diff { 
                if counts[i as usize] as usize >= threshold { 
                    results_vec.push(((current_start + i as i64) as i32, counts[i as usize] as usize)); 
                } 
            }
            progress.fetch_add(current_diff as usize, Ordering::Relaxed);
            drop(data); 
            readback_buffer.unmap();
        }
    }
    results_vec.sort_by(|a, b| b.1.cmp(&a.1)); 
    results_vec
}

const WGSL_SHADER: &str = r#"
struct Config { start_seed: i32, total_tasks: u32, chunk_size: u32, threshold: u32 }
@group(0) @binding(0) var<uniform> config: Config;
@group(0) @binding(1) var<storage, read_write> results: array<u32>;
@group(0) @binding(2) var<storage, read_write> bb: array<u32>;
@group(0) @binding(3) var<storage, read_write> mine_dirs: array<u32>;
@group(0) @binding(4) var<storage, read_write> mine_dir_counts: array<u32>;
var<private> wg_seed_array: array<i32, 56>; 
var<private> wg_inext: i32; 
var<private> wg_inextp: i32;
const base_around_50 = array<vec2<i32>, 441>(
    vec2<i32>(0,0), vec2<i32>(0,-1), vec2<i32>(-1,-1), vec2<i32>(-1,0), vec2<i32>(-1,1), vec2<i32>(0,1), vec2<i32>(1,1), vec2<i32>(1,0), vec2<i32>(1,-1), vec2<i32>(1,-2), vec2<i32>(0,-2), vec2<i32>(-1,-2), vec2<i32>(-2,-2), vec2<i32>(-2,-1), vec2<i32>(-2,0), vec2<i32>(-2,1),
    vec2<i32>(-2,2), vec2<i32>(-1,2), vec2<i32>(0,2), vec2<i32>(1,2), vec2<i32>(2,2), vec2<i32>(2,1), vec2<i32>(2,0), vec2<i32>(2,-1), vec2<i32>(2,-2), vec2<i32>(2,-3), vec2<i32>(1,-3), vec2<i32>(0,-3), vec2<i32>(-1,-3), vec2<i32>(-2,-3), vec2<i32>(-3,-3), vec2<i32>(-3,-2),
    vec2<i32>(-3,-1), vec2<i32>(-3,0), vec2<i32>(-3,1), vec2<i32>(-3,2), vec2<i32>(-3,3), vec2<i32>(-2,3), vec2<i32>(-1,3), vec2<i32>(0,3), vec2<i32>(1,3), vec2<i32>(2,3), vec2<i32>(3,3), vec2<i32>(3,2), vec2<i32>(3,1), vec2<i32>(3,0), vec2<i32>(3,-1), vec2<i32>(3,-2),
    vec2<i32>(3,-3), vec2<i32>(3,-4), vec2<i32>(2,-4), vec2<i32>(1,-4), vec2<i32>(0,-4), vec2<i32>(-1,-4), vec2<i32>(-2,-4), vec2<i32>(-3,-4), vec2<i32>(-4,-4), vec2<i32>(-4,-3), vec2<i32>(-4,-2), vec2<i32>(-4,-1), vec2<i32>(-4,0), vec2<i32>(-4,1), vec2<i32>(-4,2), vec2<i32>(-4,3),
    vec2<i32>(-4,4), vec2<i32>(-3,4), vec2<i32>(-2,4), vec2<i32>(-1,4), vec2<i32>(0,4), vec2<i32>(1,4), vec2<i32>(2,4), vec2<i32>(3,4), vec2<i32>(4,4), vec2<i32>(4,3), vec2<i32>(4,2), vec2<i32>(4,1), vec2<i32>(4,0), vec2<i32>(4,-1), vec2<i32>(4,-2), vec2<i32>(4,-3),
    vec2<i32>(4,-4), vec2<i32>(4,-5), vec2<i32>(3,-5), vec2<i32>(2,-5), vec2<i32>(1,-5), vec2<i32>(0,-5), vec2<i32>(-1,-5), vec2<i32>(-2,-5), vec2<i32>(-3,-5), vec2<i32>(-4,-5), vec2<i32>(-5,-5), vec2<i32>(-5,-4), vec2<i32>(-5,-3), vec2<i32>(-5,-2), vec2<i32>(-5,-1), vec2<i32>(-5,0),
    vec2<i32>(-5,1), vec2<i32>(-5,2), vec2<i32>(-5,3), vec2<i32>(-5,4), vec2<i32>(-5,5), vec2<i32>(-4,5), vec2<i32>(-3,5), vec2<i32>(-2,5), vec2<i32>(-1,5), vec2<i32>(0,5), vec2<i32>(1,5), vec2<i32>(2,5), vec2<i32>(3,5), vec2<i32>(4,5), vec2<i32>(5,5), vec2<i32>(5,4),
    vec2<i32>(5,3), vec2<i32>(5,2), vec2<i32>(5,1), vec2<i32>(5,0), vec2<i32>(5,-1), vec2<i32>(5,-2), vec2<i32>(5,-3), vec2<i32>(5,-4), vec2<i32>(5,-5), vec2<i32>(5,-6), vec2<i32>(4,-6), vec2<i32>(3,-6), vec2<i32>(2,-6), vec2<i32>(1,-6), vec2<i32>(0,-6), vec2<i32>(-1,-6),
    vec2<i32>(-2,-6), vec2<i32>(-3,-6), vec2<i32>(-4,-6), vec2<i32>(-5,-6), vec2<i32>(-6,-6), vec2<i32>(-6,-5), vec2<i32>(-6,-4), vec2<i32>(-6,-3), vec2<i32>(-6,-2), vec2<i32>(-6,-1), vec2<i32>(-6,0), vec2<i32>(-6,1), vec2<i32>(-6,2), vec2<i32>(-6,3), vec2<i32>(-6,4), vec2<i32>(-6,5),
    vec2<i32>(-6,6), vec2<i32>(-5,6), vec2<i32>(-4,6), vec2<i32>(-3,6), vec2<i32>(-2,6), vec2<i32>(-1,6), vec2<i32>(0,6), vec2<i32>(1,6), vec2<i32>(2,6), vec2<i32>(3,6), vec2<i32>(4,6), vec2<i32>(5,6), vec2<i32>(6,6), vec2<i32>(6,5), vec2<i32>(6,4), vec2<i32>(6,3),
    vec2<i32>(6,2), vec2<i32>(6,1), vec2<i32>(6,0), vec2<i32>(6,-1), vec2<i32>(6,-2), vec2<i32>(6,-3), vec2<i32>(6,-4), vec2<i32>(6,-5), vec2<i32>(6,-6), vec2<i32>(6,-7), vec2<i32>(5,-7), vec2<i32>(4,-7), vec2<i32>(3,-7), vec2<i32>(2,-7), vec2<i32>(1,-7), vec2<i32>(0,-7),
    vec2<i32>(-1,-7), vec2<i32>(-2,-7), vec2<i32>(-3,-7), vec2<i32>(-4,-7), vec2<i32>(-5,-7), vec2<i32>(-6,-7), vec2<i32>(-7,-7), vec2<i32>(-7,-6), vec2<i32>(-7,-5), vec2<i32>(-7,-4), vec2<i32>(-7,-3), vec2<i32>(-7,-2), vec2<i32>(-7,-1), vec2<i32>(-7,0), vec2<i32>(-7,1), vec2<i32>(-7,2),
    vec2<i32>(-7,3), vec2<i32>(-7,4), vec2<i32>(-7,5), vec2<i32>(-7,6), vec2<i32>(-7,7), vec2<i32>(-6,7), vec2<i32>(-5,7), vec2<i32>(-4,7), vec2<i32>(-3,7), vec2<i32>(-2,7), vec2<i32>(-1,7), vec2<i32>(0,7), vec2<i32>(1,7), vec2<i32>(2,7), vec2<i32>(3,7), vec2<i32>(4,7),
    vec2<i32>(5,7), vec2<i32>(6,7), vec2<i32>(7,7), vec2<i32>(7,6), vec2<i32>(7,5), vec2<i32>(7,4), vec2<i32>(7,3), vec2<i32>(7,2), vec2<i32>(7,1), vec2<i32>(7,0), vec2<i32>(7,-1), vec2<i32>(7,-2), vec2<i32>(7,-3), vec2<i32>(7,-4), vec2<i32>(7,-5), vec2<i32>(7,-6),
    vec2<i32>(7,-7), vec2<i32>(7,-8), vec2<i32>(6,-8), vec2<i32>(5,-8), vec2<i32>(4,-8), vec2<i32>(3,-8), vec2<i32>(2,-8), vec2<i32>(1,-8), vec2<i32>(0,-8), vec2<i32>(-1,-8), vec2<i32>(-2,-8), vec2<i32>(-3,-8), vec2<i32>(-4,-8), vec2<i32>(-5,-8), vec2<i32>(-6,-8), vec2<i32>(-7,-8),
    vec2<i32>(-8,-8), vec2<i32>(-8,-7), vec2<i32>(-8,-6), vec2<i32>(-8,-5), vec2<i32>(-8,-4), vec2<i32>(-8,-3), vec2<i32>(-8,-2), vec2<i32>(-8,-1), vec2<i32>(-8,0), vec2<i32>(-8,1), vec2<i32>(-8,2), vec2<i32>(-8,3), vec2<i32>(-8,4), vec2<i32>(-8,5), vec2<i32>(-8,6), vec2<i32>(-8,7),
    vec2<i32>(-8,8), vec2<i32>(-7,8), vec2<i32>(-6,8), vec2<i32>(-5,8), vec2<i32>(-4,8), vec2<i32>(-3,8), vec2<i32>(-2,8), vec2<i32>(-1,8), vec2<i32>(0,8), vec2<i32>(1,8), vec2<i32>(2,8), vec2<i32>(3,8), vec2<i32>(4,8), vec2<i32>(5,8), vec2<i32>(6,8), vec2<i32>(7,8),
    vec2<i32>(8,8), vec2<i32>(8,7), vec2<i32>(8,6), vec2<i32>(8,5), vec2<i32>(8,4), vec2<i32>(8,3), vec2<i32>(8,2), vec2<i32>(8,1), vec2<i32>(8,0), vec2<i32>(8,-1), vec2<i32>(8,-2), vec2<i32>(8,-3), vec2<i32>(8,-4), vec2<i32>(8,-5), vec2<i32>(8,-6), vec2<i32>(8,-7),
    vec2<i32>(8,-8), vec2<i32>(8,-9), vec2<i32>(7,-9), vec2<i32>(6,-9), vec2<i32>(5,-9), vec2<i32>(4,-9), vec2<i32>(3,-9), vec2<i32>(2,-9), vec2<i32>(1,-9), vec2<i32>(0,-9), vec2<i32>(-1,-9), vec2<i32>(-2,-9), vec2<i32>(-3,-9), vec2<i32>(-4,-9), vec2<i32>(-5,-9), vec2<i32>(-6,-9),
    vec2<i32>(-7,-9), vec2<i32>(-8,-9), vec2<i32>(-9,-9), vec2<i32>(-9,-8), vec2<i32>(-9,-7), vec2<i32>(-9,-6), vec2<i32>(-9,-5), vec2<i32>(-9,-4), vec2<i32>(-9,-3), vec2<i32>(-9,-2), vec2<i32>(-9,-1), vec2<i32>(-9,0), vec2<i32>(-9,1), vec2<i32>(-9,2), vec2<i32>(-9,3), vec2<i32>(-9,4),
    vec2<i32>(-9,5), vec2<i32>(-9,6), vec2<i32>(-9,7), vec2<i32>(-9,8), vec2<i32>(-9,9), vec2<i32>(-8,9), vec2<i32>(-7,9), vec2<i32>(-6,9), vec2<i32>(-5,9), vec2<i32>(-4,9), vec2<i32>(-3,9), vec2<i32>(-2,9), vec2<i32>(-1,9), vec2<i32>(0,9), vec2<i32>(1,9), vec2<i32>(2,9),
    vec2<i32>(3,9), vec2<i32>(4,9), vec2<i32>(5,9), vec2<i32>(6,9), vec2<i32>(7,9), vec2<i32>(8,9), vec2<i32>(9,9), vec2<i32>(9,8), vec2<i32>(9,7), vec2<i32>(9,6), vec2<i32>(9,5), vec2<i32>(9,4), vec2<i32>(9,3), vec2<i32>(9,2), vec2<i32>(9,1), vec2<i32>(9,0),
    vec2<i32>(9,-1), vec2<i32>(9,-2), vec2<i32>(9,-3), vec2<i32>(9,-4), vec2<i32>(9,-5), vec2<i32>(9,-6), vec2<i32>(9,-7), vec2<i32>(9,-8), vec2<i32>(9,-9), vec2<i32>(9,-10), vec2<i32>(8,-10), vec2<i32>(7,-10), vec2<i32>(6,-10), vec2<i32>(5,-10), vec2<i32>(4,-10), vec2<i32>(3,-10),
    vec2<i32>(2,-10), vec2<i32>(1,-10), vec2<i32>(0,-10), vec2<i32>(-1,-10), vec2<i32>(-2,-10), vec2<i32>(-3,-10), vec2<i32>(-4,-10), vec2<i32>(-5,-10), vec2<i32>(-6,-10), vec2<i32>(-7,-10), vec2<i32>(-8,-10), vec2<i32>(-9,-10), vec2<i32>(-10,-10), vec2<i32>(-10,-9), vec2<i32>(-10,-8), vec2<i32>(-10,-7),
    vec2<i32>(-10,-6), vec2<i32>(-10,-5), vec2<i32>(-10,-4), vec2<i32>(-10,-3), vec2<i32>(-10,-2), vec2<i32>(-10,-1), vec2<i32>(-10,0), vec2<i32>(-10,1), vec2<i32>(-10,2), vec2<i32>(-10,3), vec2<i32>(-10,4), vec2<i32>(-10,5), vec2<i32>(-10,6), vec2<i32>(-10,7), vec2<i32>(-10,8), vec2<i32>(-10,9),
    vec2<i32>(-10,10), vec2<i32>(-9,10), vec2<i32>(-8,10), vec2<i32>(-7,10), vec2<i32>(-6,10), vec2<i32>(-5,10), vec2<i32>(-4,10), vec2<i32>(-3,10), vec2<i32>(-2,10), vec2<i32>(-1,10), vec2<i32>(0,10), vec2<i32>(1,10), vec2<i32>(2,10), vec2<i32>(3,10), vec2<i32>(4,10), vec2<i32>(5,10),
    vec2<i32>(6,10), vec2<i32>(7,10), vec2<i32>(8,10), vec2<i32>(9,10), vec2<i32>(10,10), vec2<i32>(10,9), vec2<i32>(10,8), vec2<i32>(10,7), vec2<i32>(10,6), vec2<i32>(10,5), vec2<i32>(10,4), vec2<i32>(10,3), vec2<i32>(10,2), vec2<i32>(10,1), vec2<i32>(10,0), vec2<i32>(10,-1),
    vec2<i32>(10,-2), vec2<i32>(10,-3), vec2<i32>(10,-4), vec2<i32>(10,-5), vec2<i32>(10,-6), vec2<i32>(10,-7), vec2<i32>(10,-8), vec2<i32>(10,-9), vec2<i32>(10,-10)
);
fn mul_u32_to_u64(a: u32, b: u32) -> vec2<u32> {
    let a_lo = a & 0xFFFFu; let a_hi = a >> 16u; let b_lo = b & 0xFFFFu; let b_hi = b >> 16u;
    let p0 = a_lo * b_lo; let p3 = a_hi * b_hi; let cross = (a_hi * b_lo) + (a_lo * b_hi);
    var cross_carry = 0u; if (cross < (a_hi * b_lo)) { cross_carry = 1u; }
    var res_lo = p0 + ((cross & 0xFFFFu) << 16u); var carry = 0u; if (res_lo < p0) { carry = 1u; }
    return vec2<u32>(res_lo, p3 + (cross >> 16u) + (cross_carry << 16u) + carry);
}
fn div_2_31_minus_1(lo: u32, hi: u32) -> u32 {
    let shift = (hi << 1u) | (lo >> 31u); var sum_lo = lo + shift; var carry = 0u; if (sum_lo < lo) { carry = 1u; }
    let o = sum_lo; sum_lo = sum_lo + 1u; if (sum_lo < o) { carry = carry + 1u; }
    return ((hi + carry) << 1u) | (sum_lo >> 31u);
}
fn init_dotnet_random(seed: i32) {
    var subtraction: i32; if (seed == -2147483648) { subtraction = 2147483647; } else { subtraction = abs(seed); }
    var mj = 161803398 - subtraction; wg_seed_array[55] = mj; var mk = 1;
    for (var i = 1u; i < 55u; i++) { let ii = (21u * i) % 55u; wg_seed_array[ii] = mk; mk = mj - mk; if (mk < 0) { mk += 2147483647; } mj = wg_seed_array[ii]; }
    for (var j = 1u; j < 5u; j++) { for (var k = 1u; k < 56u; k++) { let idx = 1u + (k + 30u) % 55u; var val = wg_seed_array[k] - wg_seed_array[idx]; if (val < 0) { val += 2147483647; } wg_seed_array[k] = val; } }
    wg_inext = 0; wg_inextp = 31;
}
fn next_range(min_val_in: i32, max_val_in: i32) -> i32 {
    var min_val = min_val_in; var max_val = max_val_in; if (min_val > max_val) { let tmp = min_val; min_val = max_val; max_val = tmp; }
    let num_uint = u32(max_val) - u32(min_val); if (num_uint <= 1u) { return min_val; }
    wg_inext++; if (wg_inext >= 56) { wg_inext = 1; } wg_inextp++; if (wg_inextp >= 56) { wg_inextp = 1; }
    var num = wg_seed_array[u32(wg_inext)] - wg_seed_array[u32(wg_inextp)];
    if (num < 0) { num += 2147483647; } wg_seed_array[u32(wg_inext)] = num;
    let n64 = mul_u32_to_u64(u32(num), num_uint); return i32(u32(min_val) + div_2_31_minus_1(n64.x, n64.y));
}
fn get_idx(t: u32, word_idx: u32, task: u32) -> u32 { return (t * 1152u + word_idx) * config.chunk_size + task; }
fn apply_mask(task: u32, word_idx: u32, mask: u32, t_target: u32) {
    if (mask == 0u) { return; }
    for (var t = 1u; t <= 15u; t++) { let idx = get_idx(t, word_idx, task); if (t == t_target) { bb[idx] |= mask; } else { bb[idx] &= ~mask; } }
}
fn get_cpu_neighbor(key: i32, out_keys: ptr<function, array<u32, 8>>) -> i32 {
    var count = 0;
    let dirs = array<i32, 8>(6, 4, 7, 5, 1, 2, 3, 0);
    for (var d = 0; d < 8; d++) {
        let dir = dirs[d]; var n = -1;
        if (dir == 0) { n = key + 192; if (n <= 0 || n >= 36864) { n = -1; } }
        else if (dir == 1) { n = key - 192; if (n <= 0 || n >= 36864) { n = -1; } }
        else if (dir == 2) { n = key - 1; if (n < 0 || n >= 36864 || (key/192) != (n/192)) { n = -1; } }
        else if (dir == 3) { n = key + 1; if (n < 0 || n >= 36864 || (key/192) != (n/192)) { n = -1; } }
        else if (dir == 4) { let t = key - 1; if (t >= 0 && t < 36864 && (key/192) == (t/192)) { n = t - 192; if (n <= 0 || n >= 36864) { n = -1; } } }
        else if (dir == 5) { let t = key + 1; if (t >= 0 && t < 36864 && (key/192) == (t/192)) { n = t - 192; if (n <= 0 || n >= 36864) { n = -1; } } }
        else if (dir == 6) { let t = key - 1; if (t >= 0 && t < 36864 && (key/192) == (t/192)) { n = t + 192; if (n <= 0 || n >= 36864) { n = -1; } } }
        else if (dir == 7) { let t = key + 1; if (t >= 0 && t < 36864 && (key/192) == (t/192)) { n = t + 192; if (n <= 0 || n >= 36864) { n = -1; } } }
        if (n != -1) { (*out_keys)[count] = u32(n); count++; }
    } return count;
}
fn make_mine_dir(task: u32, fx: i32, fy: i32) {
    var i = 0; var num = next_range(0, 192); let num2 = num; next_range(0, 192);
    var fuse = 0; var cc = mine_dir_counts[task];
    while (i < 192 && fuse < 3000) { fuse++;
        if (next_range(0, 100) < 10) { i--; } else { i++; }
        if (num2 > 96) { num += next_range(-fy, fx); } else { num += next_range(-fx, fy); }
        if (i >= 0 && i < 192 && num >= 0 && num < 192) { 
            let key = u32(num * 192 + i);
            if (key > 0u && cc < 2048u) { mine_dirs[cc * config.chunk_size + task] = key; cc++; } 
        }
    } mine_dir_counts[task] = cc;
}
fn bb_dilate_8way(task: u32, t_src: u32, t_dst: u32) {
    var pr = array<u32, 6>(0u, 0u, 0u, 0u, 0u, 0u);
    for (var y = 0u; y < 192u; y++) {
        let b = y * 6u;
        var cr = array<u32, 6>(bb[get_idx(t_src, b, task)], bb[get_idx(t_src, b+1u, task)], bb[get_idx(t_src, b+2u, task)], bb[get_idx(t_src, b+3u, task)], bb[get_idx(t_src, b+4u, task)], bb[get_idx(t_src, b+5u, task)]);
        var nr = array<u32, 6>(0u, 0u, 0u, 0u, 0u, 0u);
        if (y < 191u) { let nb = (y + 1u) * 6u; nr = array<u32, 6>(bb[get_idx(t_src, nb, task)], bb[get_idx(t_src, nb+1u, task)], bb[get_idx(t_src, nb+2u, task)], bb[get_idx(t_src, nb+3u, task)], bb[get_idx(t_src, nb+4u, task)], bb[get_idx(t_src, nb+5u, task)]); }
        let m0 = pr[0] | cr[0] | nr[0]; let m1 = pr[1] | cr[1] | nr[1]; let m2 = pr[2] | cr[2] | nr[2]; let m3 = pr[3] | cr[3] | nr[3]; let m4 = pr[4] | cr[4] | nr[4]; let m5 = pr[5] | cr[5] | nr[5];
        bb[get_idx(t_dst, b, task)]    = m0 | (m0 << 1u) | (m0 >> 1u) | (m1 << 31u);
        bb[get_idx(t_dst, b+1u, task)] = m1 | (m1 << 1u) | (m1 >> 1u) | (m0 >> 31u) | (m2 << 31u);
        bb[get_idx(t_dst, b+2u, task)] = m2 | (m2 << 1u) | (m2 >> 1u) | (m1 >> 31u) | (m3 << 31u);
        bb[get_idx(t_dst, b+3u, task)] = m3 | (m3 << 1u) | (m3 >> 1u) | (m2 >> 31u) | (m4 << 31u);
        bb[get_idx(t_dst, b+4u, task)] = m4 | (m4 << 1u) | (m4 >> 1u) | (m3 >> 31u) | (m5 << 31u);
        bb[get_idx(t_dst, b+5u, task)] = m5 | (m5 << 1u) | (m5 >> 1u) | (m4 >> 31u);
        pr = cr;
    }
}
fn get_ctype_mask(task: u32, word_idx: u32, ctype: i32) -> u32 {
    if (ctype == 0) { return 0xFFFFFFFFu; }
    let fs = bb[get_idx(1u, word_idx, task)] | bb[get_idx(6u, word_idx, task)];
    let bp = bb[get_idx(16u, word_idx, task)] | bb[get_idx(17u, word_idx, task)];
    if (ctype == 1) { return fs & ~bp; }
    if (ctype == 2) { return fs | bb[get_idx(7u, word_idx, task)]; }
    if (ctype == 3) {
        let bad = bb[get_idx(13u, word_idx, task)] | bb[get_idx(14u, word_idx, task)] | bb[get_idx(15u, word_idx, task)] | bb[get_idx(10u, word_idx, task)] | bb[get_idx(11u, word_idx, task)];
        return ~(bad | (bb[get_idx(12u, word_idx, task)] & ~bp));
    } return 0xFFFFFFFFu;
}
fn out_line(task: u32, src: u32, t_target: u32, w: i32, lv: i32, maxcount: i32, ctype: i32) {
    for (var i = 0u; i < 1152u; i++) { bb[get_idx(18u, i, task)] = bb[get_idx(src, i, task)]; }
    if (lv >= 100 && maxcount <= 0) {
        var skip_count = 0u;
        for (var w_idx = 1151i; w_idx >= 0i; w_idx--) {
            var mask = bb[get_idx(18u, u32(w_idx), task)];
            if (w_idx == 0i) { mask &= ~1u; }
            if (mask == 0u) { continue; }
            for (var bit: i32 = 31; bit >= 0; bit--) {
                if ((mask & (1u << u32(bit))) != 0u) {
                    let key = u32(w_idx) * 32u + u32(bit);
                    let cx = i32(key % 192u); let cy = i32(key / 192u);
                    let min_x = max(0, cx - w); let max_x = min(191, cx + w);
                    let min_y = max(0, cy - w); let max_y = min(191, cy + w);
                    let valid_points = (max_x - min_x + 1) * (max_y - min_y + 1);
                    for (var k = 0; k < valid_points; k++) { next_range(0, 100); }
                }
            }
        }
        
        var cur_t = 18u; var next_t = 19u;
        bb[get_idx(18u, 0u, task)] &= ~1u; 
        for (var step = 0; step < w; step++) { bb_dilate_8way(task, cur_t, next_t); let tmp = cur_t; cur_t = next_t; next_t = tmp; }
        for (var i = 0u; i < 1152u; i++) {
            let m = (bb[get_idx(cur_t, i, task)] & ~bb[get_idx(src, i, task)]) & get_ctype_mask(task, i, ctype);
            apply_mask(task, i, m, t_target);
        }
    } else {
        let limit = u32((w * 2 + 1) * (w * 2 + 1));
        var current_max = maxcount;
        
        for (var word: i32 = 1151; word >= 0; word--) {
            var mask = bb[get_idx(18u, u32(word), task)];
            if (word == 0) { mask &= ~1u; }
            if (mask == 0u) { continue; }
            for (var bit: i32 = 31; bit >= 0; bit--) {
                if ((mask & (1u << u32(bit))) != 0u) {
                    let key = u32(word) * 32u + u32(bit);
                    let cx = i32(key % 192u); let cy = i32(key / 192u);
                    for (var i = 0u; i < limit; i++) {
                        let d = base_around_50[i];
                        let nx = cx + d.x; let ny = cy + d.y;
                        if (nx >= 0 && nx < 192 && ny >= 0 && ny < 192) {
                            if (next_range(0, 100) > lv) { continue; }
                            let n_key = u32(ny) * 192u + u32(nx);
                            let n_word = n_key / 32u; let n_bit = 1u << (n_key % 32u);
                            let c_mask = get_ctype_mask(task, n_word, ctype);
                            if ((c_mask & n_bit) != 0u) {
                                apply_mask(task, n_word, n_bit, t_target);
                                if (maxcount > 0) {
                                    current_max--;
                                    if (current_max == 0) { break; }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
fn optimize(task: u32, def: u32, opt_min: i32, opt_max: i32, maxcount: i32, ctype: i32) {
    for (var iter = 0; iter < maxcount; iter++) {
        bb_dilate_8way(task, def, 18u);
        for (var i = 0u; i < 1152u; i++) {
            let mask = bb[get_idx(18u, i, task)] & get_ctype_mask(task, i, ctype);
            var f_mask = 0u;
            if (mask != 0u) {
                let y = i / 6u; let xw = i % 6u; let c = bb[get_idx(def, i, task)];
                var cl = c >> 1u; if (xw < 5u) { cl |= bb[get_idx(def, i+1u, task)] << 31u; }
                var cr = c << 1u; if (xw > 0u) { cr |= bb[get_idx(def, i-1u, task)] >> 31u; }
                
                var u = 0u; var ul = 0u; var ur = 0u;
                if (y > 0u) { 
                    let ui = i - 6u; 
                    var u_mask = 0xFFFFFFFFu; if (ui == 0u) { u_mask = 0xFFFFFFFEu; }
                    u = bb[get_idx(def, ui, task)] & u_mask; 
                    ul = u >> 1u; if (xw < 5u) { ul |= bb[get_idx(def, ui+1u, task)] << 31u; } 
                    ur = u << 1u; if (xw > 0u) { ur |= bb[get_idx(def, ui-1u, task)] >> 31u; } 
                }
                var d = 0u; var dl = 0u; var dr = 0u;
                if (y < 191u) { 
                    let di = i + 6u; d = bb[get_idx(def, di, task)]; 
                    dl = d >> 1u; if (xw < 5u) { dl |= bb[get_idx(def, di+1u, task)] << 31u; } 
                    dr = d << 1u; if (xw > 0u) { dr |= bb[get_idx(def, di-1u, task)] >> 31u; } 
                }
                for (var b = 0u; b < 32u; b++) {
                    let bit = 1u << b;
                    if ((mask & bit) != 0u) {
                        var count = 0;
                        if ((cl & bit) != 0u) { count++; } if ((cr & bit) != 0u) { count++; }
                        if ((u & bit) != 0u) { count++; } if ((ul & bit) != 0u) { count++; } if ((ur & bit) != 0u) { count++; }
                        if ((d & bit) != 0u) { count++; } if ((dl & bit) != 0u) { count++; } if ((dr & bit) != 0u) { count++; }
                        if (count >= opt_min && count <= opt_max) { f_mask |= bit; }
                    }
                }
            } bb[get_idx(19u, i, task)] = f_mask;
        }
        for (var i = 0u; i < 1152u; i++) { apply_mask(task, i, bb[get_idx(19u, i, task)], def); }
    }
}
fn random_and_expand(task: u32, def: u32, rcount: i32, ecount: i32, expand_lv: i32, opt_lv: i32, opt_c: i32, ctype: i32, ectype: i32) {
    let rc = max(1, rcount);
    for (var k = 0; k < rc; k++) {
        let x = u32(next_range(0, 192)); let y = u32(next_range(0, 192));
        let i = y * 6u + x / 32u; let bit = 1u << (x % 32u);
        if ((get_ctype_mask(task, i, ctype) & bit) != 0u) { apply_mask(task, i, bit, def); }
    }
    
    var flag = true;
    for (var iter = 0; iter < ecount; iter++) {
        if (flag) {
            for (var key = 0u; key < 36864u; key++) {
                let w = key / 32u; let b = key % 32u;
                if ((bb[get_idx(def, w, task)] & (1u << b)) != 0u) {
                    if (next_range(0, 100) <= expand_lv) {
                        var n_keys = array<u32, 8>(0u,0u,0u,0u,0u,0u,0u,0u);
                        let n_count = get_cpu_neighbor(i32(key), &n_keys);
                        for (var d = 0; d < n_count; d++) {
                            let nw = n_keys[d] / 32u; let n_bit = 1u << (n_keys[d] % 32u);
                            if ((get_ctype_mask(task, nw, ectype) & n_bit) != 0u) { apply_mask(task, nw, n_bit, def); }
                        }
                    }
                }
            } flag = false;
        } else {
            for (var key_i = 0u; key_i < 36864u; key_i++) {
                let key = 36863u - key_i;
                let w = key / 32u; let b = key % 32u;
                if ((bb[get_idx(def, w, task)] & (1u << b)) != 0u) {
                    if (next_range(0, 100) <= expand_lv) {
                        var n_keys = array<u32, 8>(0u,0u,0u,0u,0u,0u,0u,0u);
                        let n_count = get_cpu_neighbor(i32(key), &n_keys);
                        for (var d = 0; d < n_count; d++) {
                            let nw = n_keys[d] / 32u; let n_bit = 1u << (n_keys[d] % 32u);
                            if ((get_ctype_mask(task, nw, ectype) & n_bit) != 0u) { apply_mask(task, nw, n_bit, def); }
                        }
                    }
                }
            } flag = true;
        }
    }
    if (opt_lv > 0) { optimize(task, def, opt_lv, opt_c, 1, ectype); }
}
fn random_line_from_mine_dir(task: u32, w: i32, size: i32, def: u32, ctype: i32) {
    let cc = mine_dir_counts[task]; if (cc == 0u) { return; }
    var num = u32(next_range(0, i32(cc)));
    for (var i = 0; i < size; i++) {
        if (num >= cc) { num = u32(next_range(0, i32(cc))); }
        let key = mine_dirs[num * config.chunk_size + task]; num++;
        var v_keys = array<u32, 8>(0u,0u,0u,0u,0u,0u,0u,0u);
        let n_count = get_cpu_neighbor(i32(key), &v_keys);
        
        var f_key = key; if (n_count > 0) { f_key = v_keys[next_range(0, n_count)]; }
        let nw = f_key / 32u; let nbit = 1u << (f_key % 32u);
        if ((get_ctype_mask(task, nw, ctype) & nbit) != 0u) { apply_mask(task, nw, nbit, def); }
    } out_line(task, def, def, w, 4, 0, ctype);
}
fn make_map(task: u32) {
    for (var i = 0u; i < 1152u; i++) { bb[get_idx(1u, i, task)] = 0xFFFFFFFFu; for (var t = 2u; t < 20u; t++) { bb[get_idx(t, i, task)] = 0u; } }
    mine_dir_counts[task] = 0u; make_mine_dir(task, 2, 3); make_mine_dir(task, 2, 3); make_mine_dir(task, 2, 3);
    let kx = u32(next_range(76, 133)); let ky = u32(next_range(114, 133));
    if (kx < 192u && ky < 192u) { apply_mask(task, ky * 6u + kx / 32u, 1u << (kx % 32u), 6u); }
    out_line(task, 6u, 6u, 2, 100, 0, 0); out_line(task, 6u, 6u, 5, 20, 0, 0); optimize(task, 6u, 2, 4, 1, 0);
    
    for (var i = 0u; i < 1152u; i++) { bb[get_idx(16u, i, task)] = bb[get_idx(6u, i, task)]; }
    
    out_line(task, 6u, 6u, 2, 100, 0, 0);
    for (var i = 0u; i < 1152u; i++) { apply_mask(task, i, bb[get_idx(6u, i, task)], 1u); }
    for (var i = 0; i < 4; i++) {
        let num2 = next_range(57, 115); var j = 0;
        while (j < next_range(5, 15)) {
            var k = -1; if (i == 0) { k = (num2 + j) * 192; } else if (i == 1) { k = (num2 + j) * 192 + 191; } else if (i == 2) { k = num2 + j; } else { k = 191 * 192 + num2 + j; }
            if (k >= 0 && k < 36864) { bb[get_idx(17u, u32(k)/32u, task)] |= (1u << (u32(k)%32u)); } j++;
        }
    }
    
    random_and_expand(task, 6u, 20, 4, 30, 5, 3, 0, 0);
    let ss = 3; random_and_expand(task, 3u, ss - 1, 2 * ss - 1, 13 + 6 * ss, 5, 3, 1, 1);
    out_line(task, 3u, 2u, 1, 100, 0, 2); out_line(task, 2u, 2u, 1, 10 + 6 * ss, 0, 1);
    out_line(task, 2u, 4u, 4, 50 + 12 * ss, 0, 2); optimize(task, 4u, 2, 6, 1, 2);
    random_and_expand(task, 4u, 3, 3, 20, 5, 3, 1, 1); out_line(task, 4u, 5u, 4, 90, 0, 2);
    
    var i1 = 0; while (i1 < next_range(ss, ss + 2)) { let w = next_range(0, ss); let s = next_range(5 + ss, 10 + ss); random_line_from_mine_dir(task, w, s, 13u, 1); i1++; }
    var i2 = 0; let n2 = 1 + ss; while (i2 < n2) { let w = next_range(0, 1); let s = next_range(3 + ss, 5 + ss); random_line_from_mine_dir(task, w, s, 14u, 1); i2++; }
    var i3 = 0; let n3 = 1 + ss; while (i3 < n3) { let w = next_range(0, 1); let s = next_range(3 + ss, 5 + ss); random_line_from_mine_dir(task, w, s, 15u, 1); i3++; }
    
    var i4 = 0; while (i4 < next_range(1, 3)) { 
        let w_val = next_range(0, 1); let s_val = next_range(8, 16);
        var st = 12u; if (next_range(1, 3) == 1) { st = 11u; } 
        random_line_from_mine_dir(task, w_val, s_val, st, 1); i4++; 
    }
    
    var m = 1; while (m < next_range(1, 3)) { 
        var st = 12u; if (m == 1) { st = 11u; } 
        let rc = next_range(0, 3);
        random_and_expand(task, st, rc, 3, 15 + 3 * ss, 0, 3, 1, 1); m++; 
    }
    let ir1 = next_range(0, ss + 1); let ir2 = next_range(1, ss + 1); let ir3 = next_range(1, 4); random_and_expand(task, 13u, ir1, ir2, 13 + ss * ir3, 0, 3, 1, 1);
    let cr1 = next_range(0, ss + 1); let cr2 = next_range(1, ss + 1); let cr3 = next_range(1, 4); random_and_expand(task, 14u, cr1, cr2, 13 + ss * cr3, 0, 3, 1, 1);
    let sr1 = next_range(0, ss + 1); let sr2 = next_range(1, ss + 1); let sr3 = next_range(1, 4); random_and_expand(task, 15u, sr1, sr2, 13 + ss * sr3, 0, 3, 1, 1);
    random_and_expand(task, 10u, next_range(2, ss + 1), ss + 1, 13 + ss * 4, 0, 3, 1, 1);
    for (var i = 1; i < 3; i++) { var t = 12u; if (i == 1) { t = 11u; } out_line(task, t, 10u, 1, 50 + 12 * ss, 0, 1); }
    out_line(task, 13u, 10u, 1, 50 + 12 * ss, 0, 1); out_line(task, 14u, 10u, 1, 50 + 12 * ss, 0, 1); out_line(task, 15u, 10u, 1, 50 + 12 * ss, 0, 1);
    
    let oi = next_range(1, 2); out_line(task, 13u, 10u, oi, 8 + 8 * ss, 0, 1); 
    let oc = next_range(1, 2); out_line(task, 14u, 10u, oc, 8 + 8 * ss, 0, 1); 
    let os = next_range(1, 2); out_line(task, 15u, 10u, os, 8 + 8 * ss, 0, 1);
    
    for (var j = 0; j < 3; j++) { 
        var t = 10u; if (j == 1) { t = 11u; } else if (j == 2) { t = 12u; } 
        let or = next_range(1, ss); out_line(task, t, 10u, or, 8 + 8 * ss, 0, 1); 
    }
    optimize(task, 10u, 2, 9, 1, 1);
    out_line(task, 10u, 9u, 1, 30, 0, 3); out_line(task, 13u, 9u, 1, 30, 0, 3); out_line(task, 15u, 9u, 1, 30, 0, 3); out_line(task, 14u, 9u, 1, 30, 0, 3); out_line(task, 10u, 9u, 1, 30, 0, 3);
    out_line(task, 9u, 9u, 1, 5, 0, 2); optimize(task, 9u, 2, 9, 1, 2);
    random_and_expand(task, 7u, ss, 6, 33, 5, 3, 2, 2);
    var count = 0u; for (var i = 0u; i < 1152u; i++) { count += countOneBits(bb[get_idx(7u, i, task)]); }
    results[task] = count;
}
@compute @workgroup_size(128)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x; if (index >= config.total_tasks) { return; }
    init_dotnet_random(config.start_seed + i32(index)); make_map(index);
}
"#;