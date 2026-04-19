use rayon::prelude::*;
use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};
use crate::{map_maker::MapMaker, terrain::Terrain};

pub fn scan_seeds(start: i32, end: i32, size: i32, thresh: usize, prog: Arc<AtomicUsize>) -> Vec<(i32, usize)> {
    let mut res: Vec<_> = (start..=end).into_par_iter().map_init(|| MapMaker::new(0, size, size), |m, s| {
        m.reset(s); m.make_map(); let c = m.grid.iter().filter(|&&t| t == Terrain::LingSoil).count();
        prog.fetch_add(1, Ordering::Relaxed); (c >= thresh).then_some((s, c))
    }).flatten().collect();
    res.sort_unstable_by_key(|&(_, c)| std::cmp::Reverse(c)); res
}

pub fn scan_seed_list(seeds: Vec<i32>, size: i32, thresh: usize, prog: Arc<AtomicUsize>) -> Vec<(i32, usize)> {
    let mut res: Vec<_> = seeds.into_par_iter().map_init(|| MapMaker::new(0, size, size), |m, s| {
        m.reset(s); m.make_map(); let c = m.grid.iter().filter(|&&t| t == Terrain::LingSoil).count();
        prog.fetch_add(1, Ordering::Relaxed); (c >= thresh).then_some((s, c))
    }).flatten().collect();
    res.sort_unstable_by_key(|&(_, c)| std::cmp::Reverse(c)); res
}