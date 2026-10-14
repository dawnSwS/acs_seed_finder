use crate::core::{
    map_maker::{GenerationKernel, MapMaker},
    scanner_api::ScanConfig,
};
use rayon::prelude::*;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub(crate) fn scan_seeds_limited(
    start: i32,
    end: i32,
    config: &ScanConfig,
    progress: &AtomicUsize,
    cancelled: &AtomicBool,
    limit: Option<usize>,
) -> Vec<(i32, usize)> {
    if start > end || config.validate().is_err() || limit == Some(0) {
        return Vec::new();
    }
    // Widen before iterating so both i32 endpoints and the full domain are valid.
    scan(
        (i64::from(start)..=i64::from(end))
            .into_par_iter()
            .filter_map(|seed| i32::try_from(seed).ok()),
        config,
        progress,
        cancelled,
        limit,
    )
}
pub(crate) fn scan_seed_list_limited(
    list: Vec<i32>,
    config: &ScanConfig,
    progress: &AtomicUsize,
    cancelled: &AtomicBool,
    limit: Option<usize>,
) -> Vec<(i32, usize)> {
    if list.is_empty() || config.validate().is_err() || limit == Some(0) {
        return Vec::new();
    }
    scan(list.into_par_iter(), config, progress, cancelled, limit)
}

/// Continue a heterogeneous scan without collecting its remaining seed domain.
#[cfg(feature = "gpu")]
pub(crate) fn scan_seed_iterator_limited(
    seeds: impl Iterator<Item = i32> + Send,
    config: &ScanConfig,
    progress: &AtomicUsize,
    cancelled: &AtomicBool,
    limit: Option<usize>,
) -> Vec<(i32, usize)> {
    if config.validate().is_err() || limit == Some(0) {
        return Vec::new();
    }
    scan(seeds.par_bridge(), config, progress, cancelled, limit)
}

fn scan<I: ParallelIterator<Item = i32>>(
    seeds: I,
    config: &ScanConfig,
    progress: &AtomicUsize,
    cancelled: &AtomicBool,
    limit: Option<usize>,
) -> Vec<(i32, usize)> {
    seeds
        // Cancellation must stop iteration of a large range as well as map creation.
        .take_any_while(|_| !cancelled.load(Ordering::Relaxed))
        .map_init(
            || {
                MapMaker::try_new(0, config.map_size, config.map_size)
                    .ok()
                    .map(|mut map| {
                        map.set_kernel(GenerationKernel::Frontier);
                        map
                    })
            },
            |map, seed| {
                map.as_mut()
                    .and_then(|map| generate_score(map, seed, config, progress, cancelled))
            },
        )
        .flatten()
        .fold(
            || MatchCollector::new(limit),
            |mut matches, item| {
                matches.insert(item);
                matches
            },
        )
        .reduce(
            || MatchCollector::new(limit),
            |mut matches, other| {
                matches.merge(other);
                matches
            },
        )
        .into_sorted_matches()
}

fn generate_score(
    map: &mut MapMaker,
    seed: i32,
    config: &ScanConfig,
    progress: &AtomicUsize,
    cancelled: &AtomicBool,
) -> Option<(i32, usize)> {
    if cancelled.load(Ordering::Relaxed) {
        return None;
    }
    map.reset(seed);
    // Only the score is needed; reset above makes partial map state disposable.
    let count = map.score_for_scan(config.scoring_stage, config.threshold);
    // Rejected seeds are evaluated work too; keep progress independent of hits.
    progress.fetch_add(1, Ordering::Relaxed);
    count.map(|count| (seed, count))
}

/// Retain only the best requested matches during evaluation and reduction.
pub(crate) enum MatchCollector {
    All(Vec<(i32, usize)>),
    Top {
        limit: usize,
        heap: BinaryHeap<(Reverse<usize>, i32)>,
    },
}

impl MatchCollector {
    pub(crate) fn new(limit: Option<usize>) -> Self {
        match limit {
            None => Self::All(Vec::new()),
            Some(limit) => Self::Top {
                limit,
                heap: BinaryHeap::new(),
            },
        }
    }

    fn insert(&mut self, item: (i32, usize)) {
        match self {
            Self::All(matches) => matches.push(item),
            Self::Top { limit, heap } => {
                let key = (Reverse(item.1), item.0);
                if heap.len() < *limit {
                    heap.push(key);
                } else if let Some(mut worst) = heap.peek_mut() {
                    if key < *worst {
                        *worst = key;
                    }
                }
            }
        }
    }

    pub(crate) fn extend(&mut self, items: impl IntoIterator<Item = (i32, usize)>) {
        for item in items {
            self.insert(item);
        }
    }

    fn merge(&mut self, other: Self) {
        match other {
            Self::All(mut other) => match self {
                Self::All(matches) => matches.append(&mut other),
                Self::Top { .. } => self.extend(other),
            },
            Self::Top { heap, .. } => {
                self.extend(heap.into_iter().map(|(Reverse(count), seed)| (seed, count)));
            }
        }
    }

    pub(crate) fn into_sorted_matches(self) -> Vec<(i32, usize)> {
        let mut matches = match self {
            Self::All(matches) => matches,
            Self::Top { heap, .. } => heap
                .into_iter()
                .map(|(Reverse(count), seed)| (seed, count))
                .collect(),
        };
        sort_matches(&mut matches);
        matches
    }
}

/// Equal scores use ascending seeds so results do not depend on worker scheduling.
pub(crate) fn sort_matches(matches: &mut [(i32, usize)]) {
    matches.sort_unstable_by_key(|&(seed, count)| (std::cmp::Reverse(count), seed));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::map_maker::{MapSizeError, ScoringStage};

    const EMPTY_MATCHES: [(i32, usize); 0] = [];

    fn config(stage: ScoringStage) -> ScanConfig {
        ScanConfig {
            map_size: 96,
            threshold: 0,
            scoring_stage: stage,
        }
    }

    fn sequential_scores(
        seeds: &[i32],
        config: &ScanConfig,
    ) -> Result<Vec<(i32, usize)>, MapSizeError> {
        let mut map = MapMaker::try_new(0, config.map_size, config.map_size)?;
        let mut matches: Vec<_> = seeds
            .iter()
            .map(|&seed| {
                map.reset(seed);
                (seed, map.score(config.scoring_stage))
            })
            .filter(|&(_, count)| count >= config.threshold)
            .collect();
        sort_matches(&mut matches);
        Ok(matches)
    }

    #[test]
    fn top_k_merge_preserves_ties_and_duplicates() {
        let items = [(5, 9), (-4, 9), (5, 9), (2, 11), (0, 3), (7, 11)];
        let mut expected = items.to_vec();
        sort_matches(&mut expected);
        for limit in 0..=items.len() + 1 {
            let mut left = MatchCollector::new(Some(limit));
            let mut right = MatchCollector::new(Some(limit));
            left.extend(items.iter().step_by(2).copied());
            right.extend(items.iter().skip(1).step_by(2).copied());
            left.merge(right);
            assert_eq!(
                left.into_sorted_matches(),
                expected.iter().take(limit).copied().collect::<Vec<_>>()
            );
        }
        let mut all = MatchCollector::new(None);
        all.extend(items);
        assert_eq!(all.into_sorted_matches(), expected);
    }

    #[test]
    fn list_matches_sequential_scores_at_both_stages() -> Result<(), Box<dyn std::error::Error>> {
        let seeds = vec![31, -7, 31, 42, 0, i32::MAX, i32::MIN];
        let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build()?;
        // Exercise word boundaries, a scalar fallback, and a game-sized map.
        for size in [32, 64, 65, 96] {
            for stage in [ScoringStage::RawLing6, ScoringStage::Postprocessed] {
                let mut config = config(stage);
                config.map_size = size;
                let expected = sequential_scores(&seeds, &config)?;
                for limit in [None, Some(3)] {
                    let progress = AtomicUsize::new(9);
                    let actual = pool.install(|| {
                        scan_seed_list_limited(
                            seeds.clone(),
                            &config,
                            &progress,
                            &AtomicBool::new(false),
                            limit,
                        )
                    });
                    assert_eq!(progress.load(Ordering::Relaxed), 9 + seeds.len());
                    assert_eq!(
                        actual,
                        expected
                            .iter()
                            .take(limit.unwrap_or(usize::MAX))
                            .copied()
                            .collect::<Vec<_>>(),
                        "size={size}, stage={stage:?}, limit={limit:?}"
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn inclusive_range_keeps_both_signed_endpoints() -> Result<(), MapSizeError> {
        let config = config(ScoringStage::Postprocessed);
        for seeds in [[i32::MIN, i32::MIN + 1], [i32::MAX - 1, i32::MAX]] {
            let progress = AtomicUsize::new(0);
            let expected = sequential_scores(&seeds, &config)?;
            let actual = scan_seeds_limited(
                seeds[0],
                seeds[1],
                &config,
                &progress,
                &AtomicBool::new(false),
                None,
            );
            assert_eq!(actual, expected);
            assert_eq!(progress.load(Ordering::Relaxed), 2);
        }
        Ok(())
    }

    #[test]
    fn cancelled_full_domain_returns_without_iteration() {
        let progress = AtomicUsize::new(17);
        let matches = scan_seeds_limited(
            i32::MIN,
            i32::MAX,
            &config(ScoringStage::RawLing6),
            &progress,
            &AtomicBool::new(true),
            Some(10),
        );
        assert_eq!(matches, EMPTY_MATCHES);
        assert_eq!(progress.load(Ordering::Relaxed), 17);
    }

    #[test]
    fn cancellation_between_maps_preserves_state_and_completed_progress() -> Result<(), MapSizeError>
    {
        let config = config(ScoringStage::RawLing6);
        let progress = AtomicUsize::new(0);
        let cancelled = AtomicBool::new(false);
        let mut map = MapMaker::try_new(0, config.map_size, config.map_size)?;
        map.set_kernel(GenerationKernel::Frontier);
        assert!(generate_score(&mut map, 1, &config, &progress, &cancelled).is_some());
        let completed_state = map.diagnostic_state();
        cancelled.store(true, Ordering::Relaxed);
        assert!(generate_score(&mut map, 2, &config, &progress, &cancelled).is_none());
        assert_eq!(map.diagnostic_state(), completed_state);
        assert_eq!(progress.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[test]
    fn rejected_seeds_still_advance_progress() -> Result<(), MapSizeError> {
        for stage in [ScoringStage::RawLing6, ScoringStage::Postprocessed] {
            let mut config = config(stage);
            config.threshold = usize::MAX;
            let progress = AtomicUsize::new(7);
            let cancelled = AtomicBool::new(false);
            let mut reference = MapMaker::try_new(1, config.map_size, config.map_size)?;
            let expected = reference.score(stage);
            reference.reset(1);
            assert_eq!(reference.score_for_scan(stage, 0), Some(expected));
            let expected_state = reference.diagnostic_state();
            let mut map = MapMaker::try_new(0, config.map_size, config.map_size)?;
            map.set_kernel(GenerationKernel::Frontier);
            assert!(generate_score(&mut map, 1, &config, &progress, &cancelled).is_none());
            assert_eq!(map.diagnostic_state(), expected_state);
            assert_eq!(progress.load(Ordering::Relaxed), 8);
            let seeds = vec![0, 1, -1, i32::MAX, i32::MIN];
            assert_eq!(
                scan_seed_list_limited(seeds.clone(), &config, &progress, &cancelled, Some(50)),
                EMPTY_MATCHES
            );
            assert_eq!(progress.load(Ordering::Relaxed), 8 + seeds.len());
        }
        Ok(())
    }

    #[test]
    fn threshold_and_top_k_match_complete_reference_scores(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let seeds = vec![17_547_620, 31, -7, 31, 42, 0, i32::MAX, i32::MIN];
        for stage in [ScoringStage::RawLing6, ScoringStage::Postprocessed] {
            let mut config = config(stage);
            let all_scores = sequential_scores(&seeds, &config)?;
            let threshold = all_scores
                .iter()
                .map(|&(_, count)| count)
                .max()
                .ok_or("reference scores must not be empty")?;
            for threshold in [threshold, threshold + 1] {
                config.threshold = threshold;
                let expected = sequential_scores(&seeds, &config)?;
                let progress = AtomicUsize::new(0);
                let actual = scan_seed_list_limited(
                    seeds.clone(),
                    &config,
                    &progress,
                    &AtomicBool::new(false),
                    Some(3),
                );
                assert_eq!(actual, expected.into_iter().take(3).collect::<Vec<_>>());
                assert_eq!(progress.load(Ordering::Relaxed), seeds.len());
            }
        }
        Ok(())
    }

    #[test]
    fn empty_invalid_and_zero_limit_scans_leave_progress_unchanged() {
        let mut config = config(ScoringStage::Postprocessed);
        let progress = AtomicUsize::new(11);
        let cancelled = AtomicBool::new(false);
        assert_eq!(
            scan_seed_list_limited(Vec::new(), &config, &progress, &cancelled, None),
            EMPTY_MATCHES
        );
        assert_eq!(
            scan_seeds_limited(1, 0, &config, &progress, &cancelled, None),
            EMPTY_MATCHES
        );
        assert_eq!(
            scan_seed_list_limited(vec![0], &config, &progress, &cancelled, Some(0)),
            EMPTY_MATCHES
        );
        assert_eq!(
            scan_seeds_limited(0, 0, &config, &progress, &cancelled, Some(0)),
            EMPTY_MATCHES
        );
        config.map_size = 0;
        assert_eq!(
            scan_seed_list_limited(vec![0], &config, &progress, &cancelled, None),
            EMPTY_MATCHES
        );
        assert_eq!(
            scan_seeds_limited(0, 0, &config, &progress, &cancelled, None),
            EMPTY_MATCHES
        );
        assert_eq!(progress.load(Ordering::Relaxed), 11);
    }
}
