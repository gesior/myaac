use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const SHARDS: usize = 64;
const SHARD_CAP: usize = 8192;

struct Bucket {
    attempts: u32,
    last: u64,
}

pub struct RateLimiter {
    enabled: bool,
    max_attempts: u32,
    ban_minutes: u64,
    ban_secs: u64,
    shards: Vec<Mutex<HashMap<String, Bucket>>>,
}

impl RateLimiter {
    pub fn new(enabled: bool, max_attempts: u32, ban_minutes: u64) -> Self {
        let shards = (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect();
        Self {
            enabled,
            max_attempts,
            ban_minutes,
            ban_secs: ban_minutes.saturating_mul(60),
            shards,
        }
    }

    pub fn ban_message(&self) -> String {
        format!(
            "A wrong account, password or secret has been entered {} times in a row. You are unable to log into your account for the next {} minutes. Please wait.",
            self.max_attempts, self.ban_minutes
        )
    }

    /// Records a failed login. Returns true when this failure reaches the ban threshold.
    pub fn record_failure(&self, ip: &str) -> bool {
        if !self.enabled {
            return false;
        }
        let now = unix_now();
        let mut map = self.lock(ip);
        if map.len() > SHARD_CAP {
            let ban_secs = self.ban_secs;
            map.retain(|_, bucket| now.saturating_sub(bucket.last) < ban_secs);
            if map.len() > SHARD_CAP {
                map.clear();
            }
        }
        let bucket = map
            .entry(ip.to_string())
            .or_insert(Bucket { attempts: 0, last: now });
        if now.saturating_sub(bucket.last) >= self.ban_secs {
            bucket.attempts = 1;
        } else {
            bucket.attempts = bucket.attempts.saturating_add(1);
        }
        bucket.last = now;
        bucket.attempts >= self.max_attempts
    }

    pub fn reset(&self, ip: &str) {
        if !self.enabled {
            return;
        }
        self.lock(ip).remove(ip);
    }

    fn lock(&self, ip: &str) -> std::sync::MutexGuard<'_, HashMap<String, Bucket>> {
        self.shards[shard_index(ip)]
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }
}

fn shard_index(ip: &str) -> usize {
    let mut hash = 0u64;
    for byte in ip.as_bytes() {
        hash = hash.wrapping_mul(16777619).wrapping_add(u64::from(*byte));
    }
    (hash as usize) % SHARDS
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bans_when_attempts_reach_the_limit() {
        let limiter = RateLimiter::new(true, 3, 30);
        assert!(!limiter.record_failure("10.0.0.1"));
        assert!(!limiter.record_failure("10.0.0.1"));
        assert!(limiter.record_failure("10.0.0.1"));
        assert!(!limiter.record_failure("10.0.0.2"));
        limiter.reset("10.0.0.1");
        assert!(!limiter.record_failure("10.0.0.1"));
    }

    #[test]
    fn disabled_limiter_does_not_ban() {
        let limiter = RateLimiter::new(false, 1, 30);
        assert!(!limiter.record_failure("10.0.0.1"));
        assert!(!limiter.record_failure("10.0.0.1"));
    }

    #[test]
    fn zero_ban_window_expires_immediately() {
        let limiter = RateLimiter::new(true, 2, 0);
        assert!(!limiter.record_failure("1.2.3.4"));
        assert!(!limiter.record_failure("1.2.3.4"));
    }
}
