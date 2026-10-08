//! Token-bucket bandwidth limiter shared by connections.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// A token bucket measured in bytes. A rate of 0 means unlimited.
///
/// Callers may go into debt: a large chunk is accepted immediately and the
/// caller then sleeps until the debt is repaid. This keeps chunk handling
/// simple while enforcing the average rate.
#[derive(Debug)]
pub struct RateLimiter {
    rate: AtomicU64,
    state: Mutex<Bucket>,
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

/// Burst allowance in seconds of traffic.
const BURST_SECS: f64 = 0.25;

impl RateLimiter {
    pub fn new(bytes_per_sec: u64) -> Self {
        Self {
            rate: AtomicU64::new(bytes_per_sec),
            state: Mutex::new(Bucket {
                tokens: 0.0,
                last: Instant::now(),
            }),
        }
    }

    pub fn unlimited() -> Self {
        Self::new(0)
    }

    pub fn rate(&self) -> u64 {
        self.rate.load(Ordering::Relaxed)
    }

    pub fn set_rate(&self, bytes_per_sec: u64) {
        self.rate.store(bytes_per_sec, Ordering::Relaxed);
        let mut b = self.state.lock();
        b.tokens = b.tokens.min(bytes_per_sec as f64 * BURST_SECS);
        b.last = Instant::now();
    }

    /// Reserve `n` bytes; returns how long the caller must wait.
    pub fn reserve(&self, n: u64) -> Duration {
        let rate = self.rate();
        if rate == 0 {
            return Duration::ZERO;
        }
        let mut b = self.state.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(b.last).as_secs_f64();
        b.last = now;
        let cap = rate as f64 * BURST_SECS;
        b.tokens = (b.tokens + elapsed * rate as f64).min(cap);
        b.tokens -= n as f64;
        if b.tokens >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-b.tokens / rate as f64)
        }
    }

    /// Wait until `n` bytes may be consumed.
    pub async fn acquire(&self, n: u64) {
        let wait = self.reserve(n);
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_never_waits() {
        let l = RateLimiter::unlimited();
        assert!(l.reserve(1 << 30).is_zero());
    }

    #[test]
    fn enforces_average_rate() {
        let l = RateLimiter::new(1000);
        let w = l.reserve(1000);
        // No accumulated tokens: 1000 bytes at 1000 B/s ≈ 1 s of debt.
        assert!(
            w > Duration::from_millis(900) && w <= Duration::from_millis(1001),
            "{w:?}"
        );
        let w2 = l.reserve(500);
        assert!(w2 > w, "debt accumulates");
    }

    #[tokio::test]
    async fn acquire_throughput_is_limited() {
        let l = RateLimiter::new(200_000);
        let start = Instant::now();
        for _ in 0..10 {
            l.acquire(20_000).await;
        }
        let secs = start.elapsed().as_secs_f64();
        // 200 KB at 200 KB/s -> about 1 s.
        assert!(secs > 0.8 && secs < 1.5, "{secs}");
    }
}
