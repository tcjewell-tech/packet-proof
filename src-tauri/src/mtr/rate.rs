use std::collections::VecDeque;
use std::time::Duration;

const WINDOW: usize = 10;
const FLOOR_MS: u64 = 1000;
const CEILING_MS: u64 = 3000;
const BACKOFF_LOSS_THRESHOLD: f64 = 0.20;
const RECOVER_LOSS_THRESHOLD: f64 = 0.05;
const FACTOR: f64 = 1.5;

pub struct AdaptiveRate {
    interval_ms: u64,
    window: VecDeque<bool>,
}

impl AdaptiveRate {
    pub fn new() -> Self {
        Self { interval_ms: FLOOR_MS, window: VecDeque::with_capacity(WINDOW) }
    }

    pub fn current_interval(&self) -> Duration {
        Duration::from_millis(self.interval_ms)
    }

    pub fn record_result(&mut self, lost: bool) {
        self.window.push_back(lost);
        if self.window.len() > WINDOW {
            self.window.pop_front();
        }
        if self.window.len() < WINDOW {
            return;
        }

        let loss_pct = self.window.iter().filter(|&&l| l).count() as f64 / WINDOW as f64;
        if loss_pct > BACKOFF_LOSS_THRESHOLD {
            self.interval_ms = ((self.interval_ms as f64 * FACTOR) as u64).min(CEILING_MS);
        } else if loss_pct < RECOVER_LOSS_THRESHOLD {
            self.interval_ms = ((self.interval_ms as f64 / FACTOR) as u64).max(FLOOR_MS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_one_second() {
        let r = AdaptiveRate::new();
        assert_eq!(r.current_interval(), Duration::from_millis(1000));
    }

    #[test]
    fn does_not_adjust_before_window_is_full() {
        let mut r = AdaptiveRate::new();
        for _ in 0..9 {
            r.record_result(true); // 100% loss, but window isn't full yet
        }
        assert_eq!(r.current_interval(), Duration::from_millis(1000));
    }

    #[test]
    fn backs_off_once_window_full_of_high_loss() {
        let mut r = AdaptiveRate::new();
        for _ in 0..10 {
            r.record_result(true); // 100% loss > 20% threshold
        }
        assert_eq!(r.current_interval(), Duration::from_millis(1500));
    }

    #[test]
    fn backoff_is_capped_at_three_seconds() {
        let mut r = AdaptiveRate::new();
        for _ in 0..100 {
            r.record_result(true);
        }
        assert_eq!(r.current_interval(), Duration::from_millis(3000));
    }

    #[test]
    fn recovers_once_window_full_of_low_loss() {
        let mut r = AdaptiveRate::new();
        for _ in 0..10 {
            r.record_result(true); // push it up to 1500ms first
        }
        assert_eq!(r.current_interval(), Duration::from_millis(1500));
        // Recovery is a single proportional step per evaluation, evaluated on
        // every push once the window is full (not batched once per 10). As
        // `false` results displace the lingering `true`s one at a time, the
        // window is briefly >20% loss again (e.g. 9 old losses + 1 new
        // success), which re-triggers backoff up to the 3000ms ceiling
        // before the window is finally clean enough (<5% loss) to take one
        // recovery step back down: 3000 / 1.5 = 2000ms, not a jump to the
        // floor. This intentionally exercises the gradual-recovery fix
        // (single step, not "loop until convergence").
        for _ in 0..10 {
            r.record_result(false); // fully recovered window, 0% loss < 5%
        }
        assert_eq!(r.current_interval(), Duration::from_millis(2000));
    }

    #[test]
    fn floor_is_one_second() {
        let mut r = AdaptiveRate::new();
        for _ in 0..30 {
            r.record_result(false);
        }
        assert_eq!(r.current_interval(), Duration::from_millis(1000));
    }

    #[test]
    fn moderate_loss_between_thresholds_does_not_adjust() {
        let mut r = AdaptiveRate::new();
        // 10% loss: 1 lost in every 10 — above the 5% recovery bar, below
        // the 20% backoff bar, so the interval should stay put.
        for i in 0..20 {
            r.record_result(i % 10 == 0);
        }
        assert_eq!(r.current_interval(), Duration::from_millis(1000));
    }
}
