use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use crate::map_maker::MapMaker;
use crate::terrain::Terrain;
pub fn scan_seeds(
    start: i32,
    end: i32,
    map_size: i32,
    threshold: usize,
    progress: Arc<AtomicUsize>,
) -> Vec<(i32, usize)> {
    let range = start..=end;
    let mut results: Vec<_> = range.into_par_iter()
        .map_init(
            || MapMaker::new(0, map_size, map_size),
            |maker, seed_val| {
                maker.reset(seed_val);
                maker.make_map();
                
                let ling_soil_count = maker.grid.iter().filter(|&&t| t == Terrain::LingSoil).count();
                progress.fetch_add(1, Ordering::Relaxed);
                if ling_soil_count >= threshold {
                    Some((seed_val, ling_soil_count))
                } else {
                    None
                }
            }
        )
        .filter_map(|x| x)
        .collect();
        
    results.sort_by(|a, b| b.1.cmp(&a.1));
    results
}
pub fn scan_seed_list(
    seeds: Vec<i32>,
    map_size: i32,
    threshold: usize,
    progress: Arc<AtomicUsize>,
) -> Vec<(i32, usize)> {
    let mut results: Vec<_> = seeds.into_par_iter()
        .map_init(
            || MapMaker::new(0, map_size, map_size),
            |maker, seed_val| {
                maker.reset(seed_val);
                maker.make_map();
                
                let ling_soil_count = maker.grid.iter().filter(|&&t| t == Terrain::LingSoil).count();
                progress.fetch_add(1, Ordering::Relaxed);
                if ling_soil_count >= threshold {
                    Some((seed_val, ling_soil_count))
                } else {
                    None
                }
            }
        )
        .filter_map(|x| x)
        .collect();
        
    results.sort_by(|a, b| b.1.cmp(&a.1));
    results
}