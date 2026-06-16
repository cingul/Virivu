use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct SimpleRateLimiter {
    window: Duration,
    max_requests: usize,
    buckets: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl SimpleRateLimiter {
    pub fn new(window: Duration, max_requests: usize) -> Self {
        Self {
            window,
            max_requests,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    pub fn allow(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut buckets = match self.buckets.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        let queue = buckets.entry(key.to_string()).or_insert_with(VecDeque::new);
        while let Some(front) = queue.front() {
            if now.duration_since(*front) > self.window {
                let _ = queue.pop_front();
            } else {
                break;
            }
        }
        if queue.len() >= self.max_requests {
            return false;
        }
        queue.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::SimpleRateLimiter;
    use std::time::Duration;

    #[test]
    fn allows_until_limit_then_blocks() {
        let limiter = SimpleRateLimiter::new(Duration::from_secs(60), 2);
        assert!(limiter.allow("user-1"));
        assert!(limiter.allow("user-1"));
        assert!(!limiter.allow("user-1"));
    }

    #[test]
    fn buckets_are_key_isolated() {
        let limiter = SimpleRateLimiter::new(Duration::from_secs(60), 1);
        assert!(limiter.allow("user-a"));
        assert!(limiter.allow("user-b"));
        assert!(!limiter.allow("user-a"));
    }
}
