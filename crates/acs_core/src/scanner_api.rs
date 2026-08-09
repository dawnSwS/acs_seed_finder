use std::sync::{atomic::AtomicUsize, Arc};

#[derive(PartialEq, Eq, Clone, Copy, Default, Debug)]
pub enum ComputeMode {
    CpuRayon,
    #[default]
    HeterogeneousPipeline,
}

pub struct ScanConfig {
    pub map_size: i32,
    pub threshold: usize,
}

// 解耦视图层与底层调度的关键桥梁
pub trait SeedScanner: Send + Sync {
    fn scan(
        &self,
        start: i32,
        end: i32,
        config: &ScanConfig,
        progress: Arc<AtomicUsize>,
    ) -> Vec<(i32, usize)>;
    fn scan_list(
        &self,
        list: Vec<i32>,
        config: &ScanConfig,
        progress: Arc<AtomicUsize>,
    ) -> Vec<(i32, usize)>;
}
