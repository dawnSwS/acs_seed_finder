use crate::core::map_maker::{MapMaker, MapSizeError, ScoringStage};
#[derive(PartialEq, Eq, Clone, Copy, Default, Debug)]
pub enum ComputeMode {
    #[default]
    CpuRayon,
    HeterogeneousPipeline,
}

pub struct ScanConfig {
    pub map_size: i32,
    pub threshold: usize,
    /// Keep the sixth expansion result distinct from the later soil treatment.
    pub scoring_stage: ScoringStage,
}

impl ScanConfig {
    /// Validate dimensions before allocating a map or starting a background job.
    pub fn validate(&self) -> Result<(), MapSizeError> {
        MapMaker::validate_dimensions(self.map_size, self.map_size)
    }
}
