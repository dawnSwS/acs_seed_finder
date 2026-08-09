use acs_core::rng::{DotNetRandom, GRandom};
pub trait RngState {
    fn next_int(&mut self, min: i32, max: i32) -> i32;
    fn next_flt(&mut self, min: f32, max: f32) -> f32;
    fn random_rate(&mut self, rate: f32) -> bool;
}
impl RngState for DotNetRandom {
    fn next_int(&mut self, min: i32, max: i32) -> i32 {
        self.next_range(min, max)
    }
    fn next_flt(&mut self, min: f32, max: f32) -> f32 {
        self.next_float(min, max)
    }
    fn random_rate(&mut self, rate: f32) -> bool {
        self.random_rate(rate)
    }
}
impl RngState for GRandom {
    fn next_int(&mut self, min: i32, max: i32) -> i32 {
        self.rand_range(min, max)
    }
    fn next_flt(&mut self, min: f32, max: f32) -> f32 {
        self.rand_float(min, max)
    }
    fn random_rate(&mut self, rate: f32) -> bool {
        (self.rand() as f64 / 4294967295.0) <= rate as f64
    }
}
