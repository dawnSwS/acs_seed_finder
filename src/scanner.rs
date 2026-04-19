use rayon::prelude::*;
use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};
use crate::{map_maker::MapMaker, terrain::Terrain};

fn scan<I: ParallelIterator<Item = i32>>(iter: I, size: i32, th: usize, prog: Arc<AtomicUsize>) -> Vec<(i32, usize)> {
    let mut res: Vec<_> = iter.map_init(|| MapMaker::new(0, size, size), |m, s| {
        m.reset(s); m.make_map();
        let c = m.grid.iter().filter(|&&t| t == Terrain::LingSoil).count();
        prog.fetch_add(1, Ordering::Relaxed);
        (c >= th).then_some((s, c))
    }).flatten().collect();
    res.sort_unstable_by_key(|&(_, c)| std::cmp::Reverse(c)); res
}

pub fn scan_seeds(s: i32, e: i32, size: i32, th: usize, p: Arc<AtomicUsize>) -> Vec<(i32, usize)> { scan((s..=e).into_par_iter(), size, th, p) }
pub fn scan_seed_list(seeds: Vec<i32>, size: i32, th: usize, p: Arc<AtomicUsize>) -> Vec<(i32, usize)> { scan(seeds.into_par_iter(), size, th, p) }