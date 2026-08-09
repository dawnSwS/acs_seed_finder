use acs_core::{map_maker::MapMaker, terrain::Terrain};
use rayon::prelude::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

pub(crate) fn scan_seeds(
    s: i32,
    e: i32,
    sz: i32,
    th: usize,
    p: Arc<AtomicUsize>,
) -> Vec<(i32, usize)> {
    scan((s..=e).into_par_iter(), sz, th, p)
}
pub(crate) fn scan_seed_list(
    l: Vec<i32>,
    sz: i32,
    th: usize,
    p: Arc<AtomicUsize>,
) -> Vec<(i32, usize)> {
    scan(l.into_par_iter(), sz, th, p)
}

fn scan<I: ParallelIterator<Item = i32>>(
    it: I,
    sz: i32,
    th: usize,
    p: Arc<AtomicUsize>,
) -> Vec<(i32, usize)> {
    let mut r: Vec<_> = it
        .map_init(
            || MapMaker::new(0, sz, sz),
            |m, s| {
                m.reset(s);
                m.make_map();
                let c = m.grid.iter().filter(|&&t| t == Terrain::LingSoil).count();
                p.fetch_add(1, Ordering::Relaxed);
                (c >= th).then_some((s, c))
            },
        )
        .flatten()
        .collect();
    r.sort_unstable_by_key(|&(_, c)| std::cmp::Reverse(c));
    r
}
