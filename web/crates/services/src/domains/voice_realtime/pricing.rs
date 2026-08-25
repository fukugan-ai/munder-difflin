#![forbid(unsafe_code)]

use md_web_contracts::domains::voice_realtime::{RealtimeCostSnapshot, RealtimeUsage};

const INPUT_USD_PER_MILLION: f64 = 32.0;
const OUTPUT_USD_PER_MILLION: f64 = 64.0;

pub fn compute_realtime_usd(usage: &RealtimeUsage) -> f64 {
    (usage.input_tokens as f64 / 1_000_000.0) * INPUT_USD_PER_MILLION
        + (usage.output_tokens as f64 / 1_000_000.0) * OUTPUT_USD_PER_MILLION
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RealtimeCostMeter {
    snapshot: RealtimeCostSnapshot,
}

impl RealtimeCostMeter {
    pub fn snapshot(&self) -> &RealtimeCostSnapshot {
        &self.snapshot
    }

    pub fn start(&mut self, now_ms: i64) {
        let cap_usd = self.snapshot.cap_usd;
        self.snapshot = RealtimeCostSnapshot {
            cap_usd,
            started_ms: Some(now_ms),
            ..RealtimeCostSnapshot::default()
        };
    }

    pub fn stop(&mut self) {
        self.snapshot.started_ms = None;
    }

    pub fn set_cap(&mut self, cap_usd: Option<f64>) {
        self.snapshot.cap_usd = cap_usd.filter(|cap| cap.is_finite() && *cap > 0.0);
        self.snapshot.over_cap = self
            .snapshot
            .cap_usd
            .is_some_and(|cap| self.snapshot.usd >= cap);
    }

    pub fn record(&mut self, usage: &RealtimeUsage, now_ms: i64) {
        if usage.input_tokens == 0 && usage.output_tokens == 0 {
            return;
        }
        self.snapshot.usd += compute_realtime_usd(usage);
        self.snapshot.input_tokens = self
            .snapshot
            .input_tokens
            .saturating_add(usage.input_tokens);
        self.snapshot.output_tokens = self
            .snapshot
            .output_tokens
            .saturating_add(usage.output_tokens);
        self.snapshot.last_activity_ms = Some(now_ms);
        self.snapshot.over_cap = self
            .snapshot
            .cap_usd
            .is_some_and(|cap| self.snapshot.usd >= cap);
    }

    pub fn is_idle(&self, threshold_ms: u64, now_ms: i64) -> bool {
        let Some(started_ms) = self.snapshot.started_ms else {
            return false;
        };
        let since = self.snapshot.last_activity_ms.unwrap_or(started_ms);
        now_ms.saturating_sub(since) >= threshold_ms as i64
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::voice_realtime::RealtimeUsage;

    use super::{RealtimeCostMeter, compute_realtime_usd};

    #[test]
    fn pricing_matches_audio_rates() {
        let usd = compute_realtime_usd(&RealtimeUsage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
        });

        assert_eq!(usd, 96.0);
    }

    #[test]
    fn cap_trips_after_recording() {
        let mut meter = RealtimeCostMeter::default();
        meter.start(0);
        meter.set_cap(Some(0.01));
        meter.record(
            &RealtimeUsage {
                input_tokens: 1_000,
                output_tokens: 0,
            },
            1,
        );

        assert!(meter.snapshot().over_cap);
    }

    #[test]
    fn idle_uses_session_start_before_first_usage() {
        let mut meter = RealtimeCostMeter::default();
        meter.start(100);

        assert!(meter.is_idle(50, 150));
    }

    #[test]
    fn stopped_meter_is_not_idle() {
        let meter = RealtimeCostMeter::default();

        assert!(!meter.is_idle(1, 10));
    }
}
