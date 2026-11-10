//! In-memory sliding-window rate limits (per-process).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::security::sha256_hex;

const DEFAULT_MAX_KEYS: usize = 50_000;

#[derive(Clone)]
pub struct RateLimitStore {
    inner: std::sync::Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    max_keys: usize,
}

impl Default for RateLimitStore {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_KEYS)
    }
}

impl RateLimitStore {
    pub fn new(max_keys: usize) -> Self {
        Self {
            inner: std::sync::Arc::new(Mutex::new(HashMap::new())),
            max_keys: max_keys.max(1),
        }
    }

    pub fn is_blocked(&self, key: &str, max: usize, window_secs: u64) -> bool {
        let Ok(mut map) = self.inner.lock() else {
            return true;
        };
        let now = Instant::now();
        let entry = map.entry(key.to_string()).or_default();
        prune_entry(entry, now, window_secs);
        entry.len() >= max
    }

    pub fn record_failure(&self, key: &str, max: usize, window_secs: u64) {
        let Ok(mut map) = self.inner.lock() else {
            return;
        };
        if !self.ensure_room(&mut map, key) {
            return;
        }
        let now = Instant::now();
        let entry = map.entry(key.to_string()).or_default();
        prune_entry(entry, now, window_secs);
        if entry.len() < max {
            entry.push(now);
        }
    }

    pub fn record_attempt(&self, key: &str, max: usize, window_secs: u64) {
        self.record_failure(key, max, window_secs);
    }

    /// Make space for `key`. Existing keys are never evicted; only empty buckets after prune are dropped.
    fn ensure_room(&self, map: &mut HashMap<String, Vec<Instant>>, key: &str) -> bool {
        if map.contains_key(key) {
            return true;
        }
        let now = Instant::now();
        map.retain(|_, entry| {
            prune_entry(entry, now, 3600);
            !entry.is_empty()
        });
        if map.len() < self.max_keys {
            return true;
        }
        false
    }

    #[cfg(test)]
    fn approx_key_count(&self) -> usize {
        self.inner.lock().map(|m| m.len()).unwrap_or(0)
    }
}

fn prune_entry(entry: &mut Vec<Instant>, now: Instant, window_secs: u64) {
    let window = Duration::from_secs(window_secs);
    entry.retain(|at| now.duration_since(*at) < window);
}

pub fn login_email_key(email: &str) -> String {
    let normalized = email.trim().to_lowercase();
    format!("login_email:{}", sha256_hex(&normalized))
}

pub fn recovery_email_key(email: &str) -> String {
    let normalized = email.trim().to_lowercase();
    format!("recovery_email:{}", sha256_hex(&normalized))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_accumulate_until_blocked() {
        let store = RateLimitStore::new(100);
        let key = "203.0.113.1";
        for _ in 0..10 {
            assert!(!store.is_blocked(key, 10, 60));
            store.record_failure(key, 10, 60);
        }
        assert!(store.is_blocked(key, 10, 60));
    }

    #[test]
    fn distinct_keys_are_isolated() {
        let store = RateLimitStore::new(100);
        for _ in 0..10 {
            store.record_failure("a", 10, 60);
        }
        assert!(store.is_blocked("a", 10, 60));
        assert!(!store.is_blocked("b", 10, 60));
    }

    #[test]
    fn window_expires_entries() {
        let store = RateLimitStore::new(100);
        for _ in 0..2 {
            store.record_failure("k", 2, 1);
        }
        assert!(store.is_blocked("k", 2, 1));
        std::thread::sleep(Duration::from_secs(2));
        assert!(!store.is_blocked("k", 2, 1));
    }

    #[test]
    fn email_key_is_hashed() {
        let key = login_email_key("User@Example.com");
        assert!(key.starts_with("login_email:"));
        assert!(!key.contains("user@"));
    }

    #[test]
    fn store_caps_key_growth() {
        let store = RateLimitStore::new(4);
        for i in 0..8 {
            store.record_failure(&format!("key-{i}"), 100, 3600);
        }
        assert!(store.approx_key_count() <= 4);
    }

    #[test]
    fn full_store_keeps_existing_buckets() {
        let store = RateLimitStore::new(2);
        for _ in 0..10 {
            store.record_failure("hot", 10, 60);
        }
        store.record_failure("other", 10, 60);
        assert!(store.is_blocked("hot", 10, 60));
        store.record_failure("rejected", 10, 60);
        assert!(store.approx_key_count() <= 2);
        assert!(store.is_blocked("hot", 10, 60));
    }
}
