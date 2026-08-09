mod cpu;
mod hetero;

use acs_core::scanner_api::{ComputeMode, ScanConfig, SeedScanner};
use std::sync::{atomic::AtomicUsize, Arc};

// 向上级提供高度封装的单体入口引擎
pub struct EngineScanner {
    pub mode: ComputeMode,
}
impl EngineScanner {
    pub fn new(mode: ComputeMode) -> Self {
        Self { mode }
    }
}

impl SeedScanner for EngineScanner {
    fn scan(
        &self,
        start: i32,
        end: i32,
        config: &ScanConfig,
        progress: Arc<AtomicUsize>,
    ) -> Vec<(i32, usize)> {
        match self.mode {
            ComputeMode::CpuRayon => {
                cpu::scan_seeds(start, end, config.map_size, config.threshold, progress)
            }
            ComputeMode::HeterogeneousPipeline => hetero::scan_seeds_heterogeneous(
                start,
                end,
                config.map_size,
                config.threshold,
                progress,
            ),
        }
    }
    fn scan_list(
        &self,
        list: Vec<i32>,
        config: &ScanConfig,
        progress: Arc<AtomicUsize>,
    ) -> Vec<(i32, usize)> {
        cpu::scan_seed_list(list, config.map_size, config.threshold, progress)
    }
}
