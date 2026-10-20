//! Rates as buckets of tokens, a copy of core/actors/limits.js (the actors'
//! slice ports it; the routes and the gateway use it until then): up to
//! `burst` at once, filled again at `rate` a second. take(n) is 0 when
//! there were enough, and otherwise takes none and says how many
//! milliseconds until there are, for Retry-After. More than `burst` at once
//! is never there (infinity).

use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct Rate {
    pub rate: f64,
    pub burst: f64,
}

pub struct Bucket {
    rate: Rate,
    tokens: f64,
    at: f64,
}

impl Bucket {
    pub fn new(rate: Rate, now: f64) -> Bucket {
        Bucket { rate, tokens: rate.burst, at: now }
    }

    fn fill(&mut self, now: f64) {
        self.tokens = self.rate.burst.min(self.tokens + ((now - self.at).max(0.0) / 1000.0) * self.rate.rate);
        self.at = now;
    }

    pub fn take(&mut self, n: f64, now: f64) -> f64 {
        self.fill(now);
        if self.tokens >= n {
            self.tokens -= n;
            return 0.0;
        }
        if n > self.rate.burst {
            return f64::INFINITY;
        }
        (((n - self.tokens) / self.rate.rate) * 1000.0).ceil()
    }

    /// Milliseconds until it's full again.
    pub fn full_in(&mut self, now: f64) -> f64 {
        self.fill(now);
        if self.tokens >= self.rate.burst {
            0.0
        } else {
            (((self.rate.burst - self.tokens) / self.rate.rate) * 1000.0).ceil()
        }
    }
}

/// Buckets by key, made as they're first needed, `max` at most. A full
/// bucket can go, since a new one would be full too; one that isn't never
/// does, so with `max` in use a new key waits until one is full again.
pub struct Buckets {
    all: HashMap<String, Bucket>,
    // The keys in the order they came, as a Map iterates.
    order: Vec<String>,
    max: usize,
}

impl Default for Buckets {
    fn default() -> Self {
        Buckets::new(1024)
    }
}

impl Buckets {
    pub fn new(max: usize) -> Buckets {
        Buckets { all: HashMap::new(), order: Vec::new(), max }
    }

    pub fn take(&mut self, key: &str, rate: Rate, n: f64, now: f64) -> f64 {
        if !self.all.contains_key(key) {
            if self.all.len() >= self.max {
                let mut soonest = f64::INFINITY;
                let mut gone = Vec::new();
                for k in &self.order {
                    let ms = self.all.get_mut(k).map_or(0.0, |b| b.full_in(now));
                    if ms == 0.0 {
                        gone.push(k.clone());
                    } else {
                        soonest = soonest.min(ms);
                    }
                }
                for k in gone {
                    self.all.remove(&k);
                    self.order.retain(|o| *o != k);
                }
                if self.all.len() >= self.max {
                    return soonest;
                }
            }
            self.all.insert(key.to_string(), Bucket::new(rate, now));
            self.order.push(key.to_string());
        }
        self.all.get_mut(key).map_or(0.0, |b| b.take(n, now))
    }

    /// Lets go of every bucket whose key starts with `prefix`.
    pub fn forget(&mut self, prefix: &str) {
        self.all.retain(|k, _| !k.starts_with(prefix));
        self.order.retain(|k| !k.starts_with(prefix));
    }

    pub fn len(&self) -> usize {
        self.all.len()
    }

    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }
}

/// Milliseconds since the epoch, as Date.now() gives them.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bucket_fills_at_its_rate() {
        let r = Rate { rate: 1.0, burst: 2.0 };
        let mut b = Bucket::new(r, 0.0);
        assert_eq!(b.take(1.0, 0.0), 0.0);
        assert_eq!(b.take(1.0, 0.0), 0.0);
        assert_eq!(b.take(1.0, 0.0), 1000.0);
        assert_eq!(b.take(1.0, 1000.0), 0.0);
        assert_eq!(b.take(3.0, 5000.0), f64::INFINITY);
    }

    #[test]
    fn buckets_keep_count_by_key_and_never_push_out_one_in_use() {
        let r = Rate { rate: 1.0, burst: 1.0 };
        let mut b = Buckets::new(2);
        assert_eq!(b.take("a", r, 1.0, 0.0), 0.0);
        assert_eq!(b.take("b", r, 1.0, 0.0), 0.0);
        assert_eq!(b.take("c", r, 1.0, 0.0), 1000.0, "both in use");
        assert_eq!(b.take("c", r, 1.0, 1000.0), 0.0, "full again, so one goes");
        b.forget("c");
        assert_eq!(b.len(), 0);
    }
}
