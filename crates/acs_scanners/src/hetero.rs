use acs_core::{
    constants::{get_base_around_50, CType},
    rng::{GMathUtl, RandomType::EmNone},
    terrain::Terrain::*,
};
use rayon::prelude::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Arc, Mutex, OnceLock,
};

struct GpuContext {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipe_o: Arc<wgpu::ComputePipeline>,
    pipe_a: Arc<wgpu::ComputePipeline>,
    pipe_c: Arc<wgpu::ComputePipeline>,
    bind_layout: Arc<wgpu::BindGroupLayout>,
}

static GPU_CTX: OnceLock<GpuContext> = OnceLock::new();

fn get_gpu_context() -> &'static GpuContext {
    GPU_CTX.get_or_init(|| {
        pollster::block_on(async {
            let instance = wgpu::Instance::default();
            let adapter = instance
                .request_adapter(&Default::default())
                .await
                .expect("Failed to request wgpu adapter");
            let (device, queue) = adapter
                .request_device(&Default::default())
                .await
                .expect("Failed to request wgpu device");
            let dev = Arc::new(device);

            // 【核心修复】后台守护轮询线程，防止高并发异步阻塞时 GPU 陷入休眠挂起
            let pd = dev.clone();
            std::thread::spawn(move || loop {
                let _ = pd.poll(wgpu::PollType::Poll);
                std::thread::sleep(std::time::Duration::from_millis(5));
            });

            let sh = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(include_str!("hetero.wgsl").into()),
            });

            let lay = Arc::new(
                dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: None,
                    entries: &[0, 1, 2, 3].map(|b| wgpu::BindGroupLayoutEntry {
                        binding: b,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: if b == 0 {
                                wgpu::BufferBindingType::Uniform
                            } else if b == 2 {
                                wgpu::BufferBindingType::Storage { read_only: true }
                            } else {
                                wgpu::BufferBindingType::Storage { read_only: false }
                            },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }),
                }),
            );

            macro_rules! pi {
                ($ep:expr) => {
                    Arc::new(
                        dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: None,
                            layout: Some(&dev.create_pipeline_layout(
                                &wgpu::PipelineLayoutDescriptor {
                                    bind_group_layouts: &[Some(&*lay)],
                                    ..Default::default()
                                },
                            )),
                            module: &sh,
                            entry_point: Some($ep),
                            cache: None,
                            compilation_options: Default::default(),
                        }),
                    )
                };
            }

            GpuContext {
                device: dev.clone(), // <-- Fix: Increment Arc reference count to prevent move
                queue: Arc::new(queue),
                pipe_o: pi!("o"),
                pipe_a: pi!("a"),
                pipe_c: pi!("c"),
                bind_layout: lay,
            }
        })
    })
}

struct ShadowPipeline {
    bb: [[u32; 1152]; 20],
    rand: GMathUtl,
    md: Vec<u32>,
}

impl ShadowPipeline {
    fn new(s: i32) -> Self {
        Self {
            bb: [[0; 1152]; 20],
            rand: GMathUtl::new(s),
            md: Vec::with_capacity(2048),
        }
    }
    #[inline]
    fn sm(&mut self, t: usize, w: usize, b: u32) {
        for i in 1..=15 {
            if i == t {
                self.bb[i][w] |= b;
            } else {
                self.bb[i][w] &= !b;
            }
        }
    }
    #[inline]
    fn ap(&mut self, t: usize, w: usize, b: u32) {
        for i in 1..=15 {
            if i == t {
                self.bb[i][w] |= b;
            } else {
                self.bb[i][w] &= !b;
            }
        }
    }
    fn cm(&self, i: usize, c: CType) -> u32 {
        let b = &self.bb;
        match c {
            CType::AllTrue => !0,
            CType::NoBorn => {
                (b[Soil as usize][i] | b[FertileSoil as usize][i])
                    & !(b[BornSpace as usize][i] | b[BornLine as usize][i])
            }
            CType::CheckCon => {
                b[Soil as usize][i] | b[FertileSoil as usize][i] | b[LingSoil as usize][i]
            }
            CType::CheckCon2 => {
                !(b[IronOre as usize][i]
                    | b[CopperOre as usize][i]
                    | b[SilverOre as usize][i]
                    | b[RockBrown as usize][i]
                    | b[RockGray as usize][i]
                    | (b[RockMarble as usize][i]
                        & !(b[BornSpace as usize][i] | b[BornLine as usize][i])))
            }
        }
    }
    #[inline]
    fn gg(k: i32, d: u8) -> i32 {
        if k < 0 {
            return -1;
        }
        match d {
            0 => {
                let n = k + 192;
                if n > 0 && n < 36864 {
                    n
                } else {
                    -1
                }
            }
            1 => {
                let n = k - 192;
                if n > 0 && n < 36864 {
                    n
                } else {
                    -1
                }
            }
            2 => {
                let n = k - 1;
                if n >= 0 && n < 36864 && k / 192 == n / 192 {
                    n
                } else {
                    -1
                }
            }
            3 => {
                let n = k + 1;
                if n >= 0 && n < 36864 && k / 192 == n / 192 {
                    n
                } else {
                    -1
                }
            }
            4 => {
                let n = Self::gg(k, 2);
                if n != -1 {
                    Self::gg(n, 1)
                } else {
                    -1
                }
            }
            5 => {
                let n = Self::gg(k, 3);
                if n != -1 {
                    Self::gg(n, 1)
                } else {
                    -1
                }
            }
            6 => {
                let n = Self::gg(k, 2);
                if n != -1 {
                    Self::gg(n, 0)
                } else {
                    -1
                }
            }
            7 => {
                let n = Self::gg(k, 3);
                if n != -1 {
                    Self::gg(n, 0)
                } else {
                    -1
                }
            }
            _ => -1,
        }
    }
    #[inline]
    fn nb(k: i32, o: &mut [u32; 8]) -> usize {
        let mut c = 0;
        for d in [6, 4, 7, 5, 1, 2, 3, 0] {
            let n = Self::gg(k, d);
            if n != -1 {
                o[c] = n as u32;
                c += 1;
            }
        }
        c
    }
    fn ol(&mut self, src: usize, tgt: usize, w: i32, lv: i32, mut mx: i32, c: CType) {
        self.bb[18] = self.bb[src];
        let (lim, b50) = (((w * 2 + 1) * (w * 2 + 1)) as usize, get_base_around_50());
        for wd in (0..1152).rev() {
            let mut m = self.bb[18][wd];
            if wd == 0 {
                m &= !1;
            }
            while m != 0 {
                let b = 31 - m.leading_zeros();
                m ^= 1 << b;
                let (cx, cy) = (
                    ((wd * 32 + b as usize) % 192) as i32,
                    ((wd * 32 + b as usize) / 192) as i32,
                );
                for &(dx, dy) in b50.iter().take(lim) {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if nx >= 0 && nx < 192 && ny >= 0 && ny < 192 {
                        if self.rand.random_range_int(0, 100, EmNone, "") <= lv {
                            let (nw, nb) = (
                                (ny * 192 + nx) as usize / 32,
                                1 << ((ny * 192 + nx) as u32 % 32),
                            );
                            if (self.cm(nw, c) & nb) != 0 {
                                self.sm(tgt, nw, nb);
                                mx -= 1;
                                if mx == 0 {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    fn rld(&mut self, w: i32, sz: i32, df: usize, c: CType) {
        if self.md.is_empty() {
            return;
        }
        let mut n = self
            .rand
            .random_range_int(0, self.md.len() as i32, EmNone, "") as usize;
        for _ in 0..sz {
            if n >= self.md.len() {
                n = self
                    .rand
                    .random_range_int(0, self.md.len() as i32, EmNone, "")
                    as usize;
            }
            let mut k = self.md[n];
            n += 1;
            let mut v_k = [0u32; 8];
            let nc = Self::nb(k as i32, &mut v_k);
            if nc > 0 {
                k = v_k[self.rand.random_range_int(0, nc as i32, EmNone, "") as usize];
            }
            if k < 36864 {
                let (nw, nb) = ((k / 32) as usize, 1 << (k % 32));
                if (self.cm(nw, c) & nb) != 0 {
                    self.sm(df, nw, nb);
                }
            }
        }
        self.ol(df, df, w, 4, 0, c);
    }
    fn rex(&mut self, df: usize, r: i32, e: i32, elv: i32, c: CType, ec: CType) {
        for _ in 0..r.max(1) {
            let (x, y) = (
                self.rand.random_range_int(0, 192, EmNone, "") as usize,
                self.rand.random_range_int(0, 192, EmNone, "") as usize,
            );
            let (i, bit) = (y * 6 + x / 32, 1 << (x % 32));
            if (self.cm(i, c) & bit) != 0 {
                self.sm(df, i, bit);
            }
        }
        let mut f = true;
        for _ in 0..e {
            macro_rules! st {
                ($wi:expr, $bi:expr) => {
                    for w in $wi {
                        for b in $bi {
                            if (self.bb[df][w] & (1 << b)) != 0
                                && self.rand.random_range_int(0, 100, EmNone, "") <= elv
                            {
                                let mut nk = [0u32; 8];
                                for d in 0..Self::nb((w * 32 + b) as i32, &mut nk) {
                                    let (nw, nb) = ((nk[d] / 32) as usize, 1 << (nk[d] % 32));
                                    if (self.cm(nw, ec) & nb) != 0 {
                                        self.sm(df, nw, nb);
                                    }
                                }
                            }
                        }
                    }
                };
            }
            if f {
                st!(0..1152, 0..32);
            } else {
                st!((0..1152).rev(), (0..32).rev());
            }
            f = !f;
        }
    }
}

pub(crate) fn scan_seeds_heterogeneous(
    s: i32,
    e: i32,
    _sz: i32,
    th: usize,
    p: Arc<AtomicUsize>,
) -> Vec<(i32, usize)> {
    let tot = (e as i64 - s as i64 + 1).max(0) as u32;
    if tot == 0 {
        return vec![];
    }

    // 获取全局单例的 GPU 上下文池，从根本上解决显存卡顿与初始化阻塞
    let ctx = get_gpu_context();
    let dev = &ctx.device;
    let q = &ctx.queue;

    let (bat, bb_s) = (500usize, (500 * 1152 * 4) as u64);
    let (b_tx, b_rx) = mpsc::channel();
    use wgpu::BufferUsages as BU;

    struct B {
        cf: wgpu::Buffer,
        la: wgpu::Buffer,
        va: wgpu::Buffer,
        tm: wgpu::Buffer,
        rb: wgpu::Buffer,
        bgo: wgpu::BindGroup,
        bgc: wgpu::BindGroup,
    }

    // 【核心修复】提前构建显存对象缓冲池，供多并发线程申请重用
    for _ in 0..std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(16)
        + 2
    {
        macro_rules! bf {
            ($sz:expr, $u:expr) => {
                dev.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: $sz,
                    usage: $u,
                    mapped_at_creation: false,
                })
            };
        }
        let (cf, la, va, tm, rb) = (
            bf!(16, BU::UNIFORM | BU::COPY_DST),
            bf!(bb_s, BU::STORAGE | BU::COPY_DST | BU::COPY_SRC),
            bf!(bb_s, BU::STORAGE | BU::COPY_DST),
            bf!(bb_s, BU::STORAGE | BU::COPY_SRC | BU::COPY_DST),
            bf!(bb_s, BU::MAP_READ | BU::COPY_DST),
        );
        macro_rules! bg {
            () => {
                dev.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &ctx.bind_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: cf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: la.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: va.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: tm.as_entire_binding(),
                        },
                    ],
                })
            };
        }
        b_tx.send(B {
            bgo: bg!(),
            bgc: bg!(),
            cf,
            la,
            va,
            tm,
            rb,
        })
        .unwrap();
    }

    let b_rx = Mutex::new(b_rx);

    let mut ar: Vec<_> = (s..=e)
        .step_by(bat)
        .collect::<Vec<_>>()
        .into_par_iter()
        .flat_map(|cs| {
            let d = ((e as i64 - cs as i64 + 1) as usize).min(bat);
            let mut eng: Vec<_> = (0..d)
                .map(|i| ShadowPipeline::new((cs as i64 + i as i64) as i32))
                .collect();
            let b = b_rx.lock().unwrap().recv().unwrap();
            let (mut bla, mut bva) = (vec![0u32; d * 1152], vec![0u32; d * 1152]);

            let run_cmd = |cmd: &str, omi: u32, oma: u32, rbe: bool| -> Option<Vec<u32>> {
                q.write_buffer(&b.cf, 0, bytemuck::cast_slice(&[d as u32, omi, oma, 0]));
                let mut ec = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                if cmd == "opt" {
                    {
                        let mut pass = ec.begin_compute_pass(&Default::default());
                        pass.set_pipeline(&ctx.pipe_o);
                        pass.set_bind_group(0, &b.bgo, &[]);
                        pass.dispatch_workgroups(((d + 63) / 64) as u32, 1, 1);
                    }
                    {
                        let mut pass = ec.begin_compute_pass(&Default::default());
                        pass.set_pipeline(&ctx.pipe_a);
                        pass.set_bind_group(0, &b.bgo, &[]);
                        pass.dispatch_workgroups(((d + 63) / 64) as u32, 1, 1);
                    }
                } else {
                    let mut pass = ec.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&ctx.pipe_c);
                    pass.set_bind_group(0, &b.bgc, &[]);
                    pass.dispatch_workgroups(((d + 63) / 64) as u32, 1, 1);
                }

                if rbe {
                    let copy_size = if cmd == "c" {
                        d as u64 * 4
                    } else {
                        d as u64 * 1152 * 4
                    };
                    let map_size = (copy_size + 7) & !7;
                    ec.copy_buffer_to_buffer(
                        if cmd == "c" { &b.tm } else { &b.la },
                        0,
                        &b.rb,
                        0,
                        copy_size,
                    );
                    q.submit(Some(ec.finish()));
                    let sl = b.rb.slice(0..map_size);
                    let (t_tx, t_rx) = mpsc::channel();
                    sl.map_async(wgpu::MapMode::Read, move |v| t_tx.send(v).unwrap());
                    while t_rx.try_recv().is_err() {
                        std::thread::yield_now();
                    }
                    let data = sl.get_mapped_range();
                    let res = bytemuck::cast_slice::<_, u32>(&data[0..copy_size as usize]).to_vec();
                    drop(data);
                    b.rb.unmap();
                    Some(res)
                } else {
                    q.submit(Some(ec.finish()));
                    None
                }
            };

            let mut opt =
                |eg: &mut Vec<ShadowPipeline>, tgt: usize, mi: u32, mx: u32, ct: CType| {
                    for (i, e_inst) in eg.iter().enumerate() {
                        bla[i * 1152..(i + 1) * 1152].copy_from_slice(&e_inst.bb[tgt]);
                        for w in 0..1152 {
                            bva[i * 1152 + w] = e_inst.cm(w, ct);
                        }
                    }
                    q.write_buffer(&b.la, 0, bytemuck::cast_slice(&bla));
                    q.write_buffer(&b.va, 0, bytemuck::cast_slice(&bva));
                    let o = run_cmd("opt", mi, mx, true).unwrap();
                    for (i, e_inst) in eg.iter_mut().enumerate() {
                        for w in 0..1152 {
                            let dif = o[i * 1152 + w] & !e_inst.bb[tgt][w];
                            if dif != 0 {
                                e_inst.ap(tgt, w, dif);
                            }
                        }
                    }
                };

            // 【核心修复】异构管线尺寸固定为 192x192，在此统一其缩放尺度并消除未定义常量遮蔽
            let sc = 3;

            for e_inst in eng.iter_mut() {
                for i in 0..1152 {
                    e_inst.bb[Soil as usize][i] = !0;
                }
                for _ in 0..3 {
                    let (mut i, mut n) = (0, e_inst.rand.random_range_int(0, 192, EmNone, ""));
                    let n2 = n;
                    let _ = e_inst.rand.random_range_int(0, 192, EmNone, "");
                    while i < 192 {
                        i += if e_inst.rand.random_range_int(0, 100, EmNone, "") < 10 {
                            -1
                        } else {
                            1
                        };
                        n += if n2 > 96 {
                            e_inst.rand.random_range_int(-3, 2, EmNone, "")
                        } else {
                            e_inst.rand.random_range_int(-2, 3, EmNone, "")
                        };
                        if (0..192).contains(&i) && (0..192).contains(&n) {
                            let k = n * 192 + i;
                            if k > 0 {
                                e_inst.md.push(k as u32);
                            }
                        }
                    }
                }
                let (kx, ky) = (
                    e_inst.rand.random_range_int(76, 133, EmNone, ""),
                    e_inst.rand.random_range_int(114, 133, EmNone, ""),
                );
                if kx < 192 && ky < 192 {
                    e_inst.sm(
                        FertileSoil as usize,
                        (ky * 6 + kx / 32) as usize,
                        1 << (kx % 32),
                    );
                }
                e_inst.ol(
                    FertileSoil as usize,
                    FertileSoil as usize,
                    2,
                    100,
                    0,
                    CType::AllTrue,
                );
                e_inst.ol(
                    FertileSoil as usize,
                    FertileSoil as usize,
                    5,
                    20,
                    0,
                    CType::AllTrue,
                );
            }
            opt(&mut eng, FertileSoil as usize, 2, 4, CType::AllTrue);

            for e_inst in eng.iter_mut() {
                for i in 0..1152 {
                    let mut m = e_inst.bb[FertileSoil as usize][i];
                    if i == 0 {
                        m &= !1;
                    }
                    e_inst.bb[BornSpace as usize][i] = m;
                }
                e_inst.ol(
                    FertileSoil as usize,
                    FertileSoil as usize,
                    2,
                    100,
                    0,
                    CType::AllTrue,
                );
                for i in 0..1152 {
                    let mut m = e_inst.bb[FertileSoil as usize][i];
                    if i == 0 {
                        m &= !1;
                    }
                    if m != 0 {
                        e_inst.sm(Soil as usize, i, m);
                    }
                }
                for i in 0..4 {
                    let n2 = e_inst.rand.random_range_int(57, 115, EmNone, "");
                    let mut j = 0;
                    while j < e_inst.rand.random_range_int(5, 15, EmNone, "") {
                        let k = match i {
                            0 => (n2 + j) * 192,
                            1 => (n2 + j) * 192 + 191,
                            2 => n2 + j,
                            _ => 191 * 192 + n2 + j,
                        };
                        if k >= 0 && k < 36864 {
                            e_inst.bb[BornLine as usize][(k / 32) as usize] |=
                                1 << ((k % 192) % 32);
                        }
                        j += 1;
                    }
                }
                e_inst.rex(
                    FertileSoil as usize,
                    20,
                    4,
                    30,
                    CType::AllTrue,
                    CType::AllTrue,
                );
                e_inst.rex(
                    DDepthWater as usize,
                    sc - 1,
                    2 * sc - 1,
                    13 + 6 * sc,
                    CType::NoBorn,
                    CType::NoBorn,
                );
                e_inst.ol(
                    DDepthWater as usize,
                    DepthWater as usize,
                    1,
                    100,
                    0,
                    CType::CheckCon,
                );
                e_inst.ol(
                    DepthWater as usize,
                    DepthWater as usize,
                    1,
                    10 + 6 * sc,
                    0,
                    CType::NoBorn,
                );
                e_inst.ol(
                    DepthWater as usize,
                    ShallowWater as usize,
                    4,
                    50 + 12 * sc,
                    0,
                    CType::CheckCon,
                );
            }
            opt(&mut eng, ShallowWater as usize, 2, 6, CType::CheckCon);

            for e_inst in eng.iter_mut() {
                e_inst.rex(
                    ShallowWater as usize,
                    3,
                    3,
                    20,
                    CType::NoBorn,
                    CType::NoBorn,
                );
                e_inst.ol(
                    ShallowWater as usize,
                    Mud as usize,
                    4,
                    90,
                    0,
                    CType::CheckCon,
                );

                let mut idx1 = 0;
                while idx1 < e_inst.rand.random_range_int(sc, sc + 2, EmNone, "") {
                    // 【核心修复】防止误传闭包外未定义参数，采用循环作用域内的本地合法随机值
                    let (sz_w, sz_x) = (
                        e_inst.rand.random_range_int(0, sc, EmNone, ""),
                        e_inst.rand.random_range_int(5 + sc, 10 + sc, EmNone, ""),
                    );
                    e_inst.rld(sz_w, sz_x, IronOre as usize, CType::NoBorn);
                    idx1 += 1;
                }
                for t in [CopperOre, SilverOre] {
                    for _ in 0..1 + sc {
                        let (sz_w, sz_x) = (
                            e_inst.rand.random_range_int(0, 1, EmNone, ""),
                            e_inst.rand.random_range_int(3 + sc, 5 + sc, EmNone, ""),
                        );
                        e_inst.rld(sz_w, sz_x, t as usize, CType::NoBorn);
                    }
                }
                let mut idx4 = 0;
                while idx4 < e_inst.rand.random_range_int(1, 3, EmNone, "") {
                    let (sz_w, sz_x, t) = (
                        e_inst.rand.random_range_int(0, 1, EmNone, ""),
                        e_inst.rand.random_range_int(8, 16, EmNone, ""),
                        if e_inst.rand.random_range_int(1, 3, EmNone, "") == 1 {
                            RockGray as usize
                        } else {
                            RockMarble as usize
                        },
                    );
                    e_inst.rld(sz_w, sz_x, t, CType::NoBorn);
                    idx4 += 1;
                }

                let mut m = 1;
                while m < e_inst.rand.random_range_int(1, 3, EmNone, "") {
                    let rc = e_inst.rand.random_range_int(0, 3, EmNone, "");
                    e_inst.rex(
                        if m == 1 {
                            RockGray as usize
                        } else {
                            RockMarble as usize
                        },
                        rc,
                        3,
                        15 + 3 * sc,
                        CType::NoBorn,
                        CType::NoBorn,
                    );
                    m += 1;
                }
                for t in [IronOre, CopperOre, SilverOre] {
                    let (a1, a2, a3) = (
                        e_inst.rand.random_range_int(0, sc + 1, EmNone, ""),
                        e_inst.rand.random_range_int(1, sc + 1, EmNone, ""),
                        e_inst.rand.random_range_int(1, 4, EmNone, ""),
                    );
                    e_inst.rex(
                        t as usize,
                        a1,
                        a2,
                        13 + sc * a3,
                        CType::NoBorn,
                        CType::NoBorn,
                    );
                }
                let r = e_inst.rand.random_range_int(2, sc + 1, EmNone, "");
                e_inst.rex(
                    RockBrown as usize,
                    r,
                    sc + 1,
                    13 + sc * 4,
                    CType::NoBorn,
                    CType::NoBorn,
                );

                for t in [RockGray, RockMarble, IronOre, CopperOre, SilverOre] {
                    e_inst.ol(
                        t as usize,
                        RockBrown as usize,
                        1,
                        50 + 12 * sc,
                        0,
                        CType::NoBorn,
                    );
                }
                for t in [IronOre, CopperOre, SilverOre] {
                    let r = e_inst.rand.random_range_int(1, 2, EmNone, "");
                    e_inst.ol(
                        t as usize,
                        RockBrown as usize,
                        r,
                        8 + 8 * sc,
                        0,
                        CType::NoBorn,
                    );
                }
                for j in 0..3 {
                    let t = match j {
                        1 => RockGray,
                        2 => RockMarble,
                        _ => RockBrown,
                    };
                    let r = e_inst.rand.random_range_int(1, sc, EmNone, "");
                    e_inst.ol(
                        t as usize,
                        RockBrown as usize,
                        r,
                        8 + 8 * sc,
                        0,
                        CType::NoBorn,
                    );
                }
            }
            opt(&mut eng, RockBrown as usize, 2, 9, CType::NoBorn);

            for e_inst in eng.iter_mut() {
                for t in [RockBrown, IronOre, SilverOre, CopperOre, RockBrown] {
                    e_inst.ol(t as usize, StoneLand as usize, 1, 30, 0, CType::CheckCon2);
                }
                e_inst.ol(
                    StoneLand as usize,
                    StoneLand as usize,
                    1,
                    5,
                    0,
                    CType::CheckCon,
                );
            }
            opt(&mut eng, StoneLand as usize, 2, 9, CType::CheckCon);

            for (i, e_inst) in eng.iter_mut().enumerate() {
                e_inst.rex(
                    LingSoil as usize,
                    sc,
                    6,
                    33,
                    CType::CheckCon,
                    CType::CheckCon,
                );
                bla[i * 1152..(i + 1) * 1152].copy_from_slice(&e_inst.bb[LingSoil as usize]);
            }
            q.write_buffer(&b.la, 0, bytemuck::cast_slice(&bla));

            // 【核心修复】重命名计分数组，防止原版 "let sc = ..." 与上方的缩放常量发生同名遮蔽引发编译错误
            let counts = run_cmd("c", 0, 0, true).unwrap();

            let mut br = Vec::new();
            for i in 0..d {
                if counts[i] as usize >= th {
                    br.push((cs as i32 + i as i32, counts[i] as usize));
                }
            }
            p.fetch_add(d, Ordering::Relaxed);
            b_tx.send(b).unwrap();
            br
        })
        .collect();

    ar.sort_unstable_by_key(|a| std::cmp::Reverse(a.1));
    ar
}
