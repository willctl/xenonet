//! Rates, as buckets of tokens (core/actors/limits.js;
//! docs/design/actors/interface.md, section 2, "Limits"): up to `burst` at
//! once, filled again at `rate` a second. `take(n)` takes n and is 0 when
//! there were enough, and otherwise takes none and says how many
//! milliseconds until there are, for Retry-After. More than `burst` at once
//! is never there. The time is the caller's, in ms.

#[derive(Clone, Debug)]
pub struct Bucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    at: f64,
}

impl Bucket {
    pub fn new(rate: f64, burst: f64, now: f64) -> Self {
        Bucket { rate, burst, tokens: burst, at: now }
    }

    fn fill(&mut self, t: f64) {
        self.tokens = self.burst.min_js(self.tokens + ((t - self.at).max_js(0.0) / 1000.0) * self.rate);
        self.at = t;
    }

    pub fn take(&mut self, n: f64, now: f64) -> f64 {
        self.fill(now);
        if self.tokens >= n {
            self.tokens -= n;
            return 0.0;
        }
        if n > self.burst {
            return f64::INFINITY;
        }
        (((n - self.tokens) / self.rate) * 1000.0).ceil()
    }

    /// Milliseconds until it's full again: 0 once a new bucket would be
    /// the same as this one.
    pub fn full_in(&mut self, now: f64) -> f64 {
        self.fill(now);
        if self.tokens >= self.burst {
            0.0
        } else {
            (((self.burst - self.tokens) / self.rate) * 1000.0).ceil()
        }
    }
}

// Math.min and Math.max, which are NaN when either side is.
trait JsMath {
    fn min_js(self, o: f64) -> f64;
    fn max_js(self, o: f64) -> f64;
}

impl JsMath for f64 {
    fn min_js(self, o: f64) -> f64 {
        if self.is_nan() || o.is_nan() {
            f64::NAN
        } else {
            self.min(o)
        }
    }
    fn max_js(self, o: f64) -> f64 {
        if self.is_nan() || o.is_nan() {
            f64::NAN
        } else {
            self.max(o)
        }
    }
}

/// Buckets by key (an actor, an instance, a process), made as they're
/// first needed, `max` at most. A full bucket can go, since a new one would
/// be full too; one that isn't never does, or a caller could make new keys
/// to push out the buckets counting it. So with `max` buckets all in use, a
/// new key waits until one is full again.
pub struct Buckets {
    max: usize,
    all: Vec<(String, Bucket)>,
}

impl Buckets {
    pub fn new(max: usize) -> Self {
        Buckets { max, all: Vec::new() }
    }

    pub fn take(&mut self, key: &str, rate: f64, burst: f64, n: f64, now: f64) -> f64 {
        let i = match self.all.iter().position(|(k, _)| k == key) {
            Some(i) => i,
            None => {
                if self.all.len() >= self.max {
                    let mut soonest = f64::INFINITY;
                    self.all.retain_mut(|(_, b)| {
                        let ms = b.full_in(now);
                        if ms == 0.0 {
                            return false;
                        }
                        soonest = soonest.min_js(ms);
                        true
                    });
                    if self.all.len() >= self.max {
                        return soonest;
                    }
                }
                self.all.push((key.to_string(), Bucket::new(rate, burst, now)));
                self.all.len() - 1
            }
        };
        self.all[i].1.take(n, now)
    }

    /// Lets go of every bucket whose key starts with `prefix` (an actor
    /// gone).
    pub fn forget(&mut self, prefix: &str) {
        self.all.retain(|(k, _)| !k.starts_with(prefix));
    }

    pub fn size(&self) -> usize {
        self.all.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets() {
        let mut b = Bucket::new(2.0, 3.0, 0.0);
        assert_eq!([b.take(1.0, 0.0), b.take(1.0, 0.0), b.take(1.0, 0.0), b.take(1.0, 0.0)], [0.0, 0.0, 0.0, 500.0]);
        assert_eq!(b.take(4.0, 0.0), f64::INFINITY);
        let mut all = Buckets::new(1);
        assert_eq!(all.take("a", 1.0, 1.0, 1.0, 0.0), 0.0);
        assert_eq!(all.take("b", 1.0, 1.0, 1.0, 0.0), 1000.0, "a is in use");
        assert_eq!(all.take("b", 1.0, 1.0, 1.0, 1000.0), 0.0, "a is full again, so it goes");
        all.forget("b");
        assert_eq!(all.size(), 0);
    }
}
