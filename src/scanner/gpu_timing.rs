//! CPU-only screening of timestamp readbacks. Passing these checks is not proof
//! that a pair belongs to the current dispatch: stale pairs can be monotonic and
//! fit inside the host interval. Any detected anomaly disables accepted device
//! timings for this device context while preserving subsequent raw observations.

use serde_json::{json, Value};
use std::time::Duration;

/// Host call spans only. Poll and channel waits are disjoint parts of the wait
/// loop, while GPU execution overlaps these host spans. No per-poll logging.
#[derive(Default)]
pub(super) struct WaitStatistics {
    poll_calls: u64,
    poll_total_ms: f64,
    poll_max_ms: f64,
    poll_last_ms: f64,
    channel_wait_calls: u64,
    channel_wait_total_ms: f64,
    channel_wait_max_ms: f64,
    channel_timeouts: u64,
}

impl WaitStatistics {
    pub(super) fn observe_poll(&mut self, elapsed: Duration) {
        let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
        self.poll_calls += 1;
        self.poll_total_ms += elapsed_ms;
        self.poll_max_ms = self.poll_max_ms.max(elapsed_ms);
        self.poll_last_ms = elapsed_ms;
    }

    pub(super) fn observe_channel_wait(&mut self, elapsed: Duration, timed_out: bool) {
        let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
        self.channel_wait_calls += 1;
        self.channel_wait_total_ms += elapsed_ms;
        self.channel_wait_max_ms = self.channel_wait_max_ms.max(elapsed_ms);
        self.channel_timeouts += u64::from(timed_out);
    }

    pub(super) fn details(&self) -> Value {
        json!({
            "poll_calls":self.poll_calls,"poll_total_ms":self.poll_total_ms,
            "poll_max_ms":self.poll_max_ms,"poll_last_ms":self.poll_last_ms,
            "channel_wait_calls":self.channel_wait_calls,"channel_wait_total_ms":self.channel_wait_total_ms,
            "channel_wait_max_ms":self.channel_wait_max_ms,"channel_timeouts":self.channel_timeouts,
            "channel_requested_timeout_ms":2,
        })
    }
}

#[derive(Default)]
pub(super) struct TimestampState {
    previous_raw: Option<[u64; 2]>,
    first_invalid_reason: Option<&'static str>,
    samples: u64,
}

pub(super) struct TimestampSample {
    pub device_execution_ms: Option<f64>,
    pub details: Value,
}

impl TimestampState {
    /// `host_upper_ms` encloses submission through successful map completion.
    /// Its timer must start before `queue.submit`, which may itself block.
    pub(super) fn inspect(
        &mut self,
        raw: [u64; 2],
        period_ns: f64,
        host_upper_ms: f64,
    ) -> TimestampSample {
        self.samples = self.samples.saturating_add(1);
        let period_valid = period_ns.is_finite() && period_ns > 0.0;
        let host_valid = host_upper_ms.is_finite() && host_upper_ms >= 0.0;
        // Subtract integers first. Converting large absolute u64 timestamps to
        // f64 first would lose small, otherwise exactly representable deltas.
        let delta_ticks = raw[1].checked_sub(raw[0]);
        let candidate_ms = delta_ticks
            .filter(|_| period_valid)
            .map(|delta| delta as f64 * period_ns / 1_000_000.0)
            .filter(|duration| duration.is_finite());
        // A conservative diagnostic tolerance, not a correction to the result:
        // 1 ms clock margin, 1% relative drift margin, or two device ticks.
        let tolerance_ms = if host_valid && period_valid {
            Some(1.0_f64.max(host_upper_ms * 0.01).max(period_ns / 500_000.0))
        } else {
            None
        };
        let reason = if !period_valid {
            Some("invalid_timestamp_period")
        } else if !host_valid {
            Some("invalid_host_upper_bound")
        } else if raw[0] == 0 || raw[1] == 0 {
            Some("zero_timestamp")
        } else if raw[0] == raw[1] {
            Some("equal_timestamps")
        } else if raw[1] < raw[0] {
            // Counter width is not supplied by this API. Do not reinterpret
            // inverted/stale words as a valid wraparound interval.
            Some("reversed_or_wrapped_timestamps")
        } else if self.previous_raw == Some(raw) {
            Some("repeated_timestamp_pair")
        } else if self
            .previous_raw
            .is_some_and(|previous| raw[0] < previous[1])
        {
            // Dispatches in one context complete serially. An interval starting
            // before the preceding observed end is not safely attributable.
            Some("non_monotonic_timestamp_pair")
        } else if candidate_ms.is_none() {
            Some("non_finite_timestamp_duration")
        } else if candidate_ms.is_some_and(|duration| duration <= 0.0) {
            Some("non_positive_timestamp_duration")
        } else if candidate_ms
            .zip(tolerance_ms)
            .is_some_and(|(duration, tolerance)| duration > host_upper_ms + tolerance)
        {
            Some("duration_exceeds_host_upper_bound")
        } else {
            None
        };
        let already_invalid = self.first_invalid_reason.is_some();
        if self.first_invalid_reason.is_none() {
            self.first_invalid_reason = reason;
        }
        let device_execution_ms = if self.first_invalid_reason.is_none() {
            candidate_ms
        } else {
            None
        };
        let status = if already_invalid {
            "context_invalid_latched"
        } else if reason.is_some() {
            "rejected"
        } else {
            "sanity_passed"
        };
        let previous_raw = self.previous_raw;
        self.previous_raw = Some(raw);
        TimestampSample {
            device_execution_ms,
            details: json!({
                "status": status,
                "sample_sequence": self.samples,
                "current_sample_reason": reason,
                "context_invalid_latched": self.first_invalid_reason.is_some(),
                "first_invalid_reason": self.first_invalid_reason,
                "raw_start_tick": raw[0].to_string(),
                "raw_end_tick": raw[1].to_string(),
                "previous_raw_ticks": previous_raw.map(|pair| pair.map(|tick| tick.to_string())),
                "delta_ticks": delta_ticks.map(|ticks| ticks.to_string()),
                "timestamp_period_ns": finite_number_or_text(period_ns),
                "candidate_ms": candidate_ms,
                "host_upper_ms": finite_number_or_text(host_upper_ms),
                "host_tolerance_ms": tolerance_ms,
                "tolerance_rule": "max(1 ms, 1% of host upper bound, 2 device ticks)",
                "sanity_scope": "Rejects obvious invalid values only; cannot prove current-dispatch attribution. Any anomaly latches timing unavailable for this device context.",
                "counter_wrap_policy": "reject; effective timestamp counter width is unavailable",
            }),
        }
    }
}

fn finite_number_or_text(value: f64) -> Value {
    if value.is_finite() {
        json!(value)
    } else {
        json!(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::TimestampState;

    #[test]
    fn observed_zero_then_plausible_stale_duration_stays_unavailable() {
        let mut state = TimestampState::default();
        let first = state.inspect([0, 0], 1.0, 3379.808);
        assert_eq!(first.device_execution_ms, None);
        assert_eq!(first.details["current_sample_reason"], "zero_timestamp");
        let second = state.inspect([1_000_000_000, 3_357_296_160], 1.0, 3445.6913);
        assert_eq!(second.device_execution_ms, None);
        assert_eq!(second.details["status"], "context_invalid_latched");
        assert_eq!(second.details["candidate_ms"], 2357.29616);
        assert!(second.details["current_sample_reason"].is_null());
        assert_eq!(second.details["first_invalid_reason"], "zero_timestamp");
    }

    #[test]
    fn supplied_impossible_warmup_duration_is_rejected() {
        let mut state = TimestampState::default();
        let sample = state.inspect([1_000_000_000, 1_886_242_640], 1.0, 379.2547);
        assert_eq!(sample.device_execution_ms, None);
        assert_eq!(sample.details["candidate_ms"], 886.24264);
        assert_eq!(
            sample.details["current_sample_reason"],
            "duration_exceeds_host_upper_bound"
        );
    }

    #[test]
    fn integer_delta_preserves_small_interval_near_u64_max() {
        let mut state = TimestampState::default();
        let sample = state.inspect([u64::MAX - 100, u64::MAX - 60], 25.0, 1.0);
        assert_eq!(sample.device_execution_ms, Some(0.001));
        assert_eq!(sample.details["delta_ticks"], "40");
        assert_eq!(
            sample.details["raw_start_tick"],
            (u64::MAX - 100).to_string()
        );
        assert_eq!(sample.details["status"], "sanity_passed");
    }

    #[test]
    fn equal_reversed_and_zero_words_are_not_durations() {
        for (raw, reason) in [
            ([5, 5], "equal_timestamps"),
            ([u64::MAX - 5, 9], "reversed_or_wrapped_timestamps"),
            ([0, 9], "zero_timestamp"),
            ([5, 0], "zero_timestamp"),
        ] {
            let sample = TimestampState::default().inspect(raw, 1.0, 1.0);
            assert_eq!(sample.device_execution_ms, None);
            assert_eq!(sample.details["current_sample_reason"], reason);
        }
    }

    #[test]
    fn repeated_and_overlapping_observations_latch_context_invalid() {
        for (raw, reason) in [
            ([10, 20], "repeated_timestamp_pair"),
            ([19, 30], "non_monotonic_timestamp_pair"),
            ([5, 9], "non_monotonic_timestamp_pair"),
        ] {
            let mut state = TimestampState::default();
            assert!(state
                .inspect([10, 20], 1.0, 1.0)
                .device_execution_ms
                .is_some());
            let sample = state.inspect(raw, 1.0, 1.0);
            assert_eq!(sample.device_execution_ms, None);
            assert_eq!(sample.details["current_sample_reason"], reason);
            assert_eq!(state.inspect([50, 60], 1.0, 1.0).device_execution_ms, None);
        }
    }

    #[test]
    fn sequential_intervals_can_touch_at_one_clock_tick() {
        let mut state = TimestampState::default();
        assert!(state
            .inspect([10, 20], 80.0, 1.0)
            .device_execution_ms
            .is_some());
        assert!(state
            .inspect([20, 30], 80.0, 1.0)
            .device_execution_ms
            .is_some());
    }

    #[test]
    fn invalid_periods_and_overflow_are_preserved_but_rejected() {
        for period in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let sample = TimestampState::default().inspect([10, 20], period, 1.0);
            assert_eq!(sample.device_execution_ms, None);
            assert_eq!(
                sample.details["current_sample_reason"],
                "invalid_timestamp_period"
            );
            if !period.is_finite() {
                assert!(sample.details["timestamp_period_ns"].is_string());
            }
        }
        let sample = TimestampState::default().inspect([1, u64::MAX], f64::MAX, 1.0);
        assert_eq!(sample.device_execution_ms, None);
        assert_eq!(
            sample.details["current_sample_reason"],
            "non_finite_timestamp_duration"
        );
    }

    #[test]
    fn malformed_host_bounds_are_rejected() {
        for bound in [-1.0, f64::NAN, f64::INFINITY] {
            let sample = TimestampState::default().inspect([10, 20], 1.0, bound);
            assert_eq!(sample.device_execution_ms, None);
            assert_eq!(
                sample.details["current_sample_reason"],
                "invalid_host_upper_bound"
            );
        }
    }

    #[test]
    fn underflowed_duration_is_not_reported_as_zero_work() {
        let sample = TimestampState::default().inspect([10, 11], f64::from_bits(1), 1.0);
        assert_eq!(sample.device_execution_ms, None);
        assert_eq!(
            sample.details["current_sample_reason"],
            "non_positive_timestamp_duration"
        );
    }

    #[test]
    fn plausible_stale_pair_without_prior_anomaly_is_not_detectable() {
        // This counterexample records the limit of the screening, rather than
        // claiming to solve stale attribution through arithmetic alone.
        let sample =
            TimestampState::default().inspect([1_000_000_000, 3_357_296_160], 1.0, 3445.6913);
        assert_eq!(sample.device_execution_ms, Some(2357.29616));
        assert_eq!(sample.details["status"], "sanity_passed");
        assert!(sample.details["sanity_scope"]
            .as_str()
            .unwrap()
            .contains("cannot prove"));
    }
}
