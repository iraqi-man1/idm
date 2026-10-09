//! Throughput measurement over a sliding window.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct SpeedMeter {
    window: Duration,
    samples: VecDeque<(Instant, u64)>,
}

impl SpeedMeter {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            samples: VecDeque::new(),
        }
    }

    /// Record the cumulative byte counter at `now`.
    pub fn record(&mut self, now: Instant, total_bytes: u64) {
        // A counter that went backwards (restart) resets the window.
        if self.samples.back().is_some_and(|&(_, b)| b > total_bytes) {
            self.samples.clear();
        }
        self.samples.push_back((now, total_bytes));
        while let Some(&(t, _)) = self.samples.front() {
            if now.duration_since(t) > self.window && self.samples.len() > 2 {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    /// Bytes per second over the window.
    pub fn speed(&self) -> u64 {
        match (self.samples.front(), self.samples.back()) {
            (Some(&(t0, b0)), Some(&(t1, b1))) if t1 > t0 => {
                let dt = t1.duration_since(t0).as_secs_f64();
                if dt < 0.2 {
                    return 0;
                }
                ((b1 - b0) as f64 / dt) as u64
            }
            _ => 0,
        }
    }

    /// Speed over the most recent `span` only.
    pub fn speed_over(&self, span: Duration) -> u64 {
        let Some(&(t1, b1)) = self.samples.back() else {
            return 0;
        };
        let first = self
            .samples
            .iter()
            .find(|(t, _)| t1.duration_since(*t) <= span);
        match first {
            Some(&(t0, b0)) if t1 > t0 => {
                let dt = t1.duration_since(t0).as_secs_f64();
                if dt < 0.2 {
                    0
                } else {
                    ((b1 - b0) as f64 / dt) as u64
                }
            }
            _ => 0,
        }
    }

    pub fn reset(&mut self) {
        self.samples.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_rate() {
        let mut m = SpeedMeter::new(Duration::from_secs(3));
        let t0 = Instant::now();
        for i in 0..=10u64 {
            m.record(t0 + Duration::from_millis(i * 500), i * 50_000);
        }
        // 50 KB per 0.5 s = 100 KB/s
        let s = m.speed();
        assert!((99_000..=101_000).contains(&s), "{s}");
        let s = m.speed_over(Duration::from_secs(1));
        assert!((99_000..=101_000).contains(&s), "{s}");
    }
}
