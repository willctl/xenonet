//! Sorting as JS's `Array.prototype.sort` sorts with a compare function:
//! stable, and taking whatever the function says.
//!
//! The JS sorts by numbers with `(a, b) => b.at - a.at`, and its ports
//! compare with `partial_cmp(…).unwrap_or(Equal)`: a NaN (a time that
//! didn't parse, a field that isn't there) is then "equal" to everything,
//! which is no total order. JS sorts on regardless; Rust's own sorts may
//! panic on such a comparator ("user-provided comparison function does not
//! correctly implement a total order"). This sort never does, and for a
//! comparator that is a total order it gives the order `sort_by` gives.

use std::cmp::Ordering;

/// Sorts `v` by `cmp`, stably (a merge sort), whatever `cmp` says.
pub fn sort_by<T>(v: &mut [T], mut cmp: impl FnMut(&T, &T) -> Ordering) {
    let n = v.len();
    if n < 2 {
        return;
    }
    // Which element goes where, merged in runs of 1, 2, 4… An element of
    // the right run goes first only when it's less, so equal ones keep
    // their order.
    let mut order: Vec<usize> = (0..n).collect();
    let mut merged = order.clone();
    let mut width = 1;
    while width < n {
        let mut start = 0;
        while start < n {
            let mid = (start + width).min(n);
            let end = (start + 2 * width).min(n);
            let (mut a, mut b, mut k) = (start, mid, start);
            while a < mid && b < end {
                if cmp(&v[order[b]], &v[order[a]]) == Ordering::Less {
                    merged[k] = order[b];
                    b += 1;
                } else {
                    merged[k] = order[a];
                    a += 1;
                }
                k += 1;
            }
            merged[k..k + (mid - a)].copy_from_slice(&order[a..mid]);
            k += mid - a;
            merged[k..k + (end - b)].copy_from_slice(&order[b..end]);
            start = end;
        }
        std::mem::swap(&mut order, &mut merged);
        width *= 2;
    }
    // Each element to its place, a cycle of swaps at a time: the one at
    // order[p] goes to p.
    let mut placed = vec![false; n];
    for start in 0..n {
        let mut p = start;
        while !placed[p] {
            placed[p] = true;
            let from = order[p];
            if from == start {
                break;
            }
            v.swap(p, from);
            p = from;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A small generator, so the tests need no crate for randomness.
    fn numbers(seed: &mut u64, n: usize, nan: bool) -> Vec<f64> {
        (0..n)
            .map(|_| {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 7;
                *seed ^= *seed << 17;
                if nan && *seed % 4 == 0 {
                    f64::NAN
                } else {
                    (*seed % 50) as f64
                }
            })
            .collect()
    }

    #[test]
    fn sorts_as_sort_by_does_for_a_total_order() {
        let mut seed = 0x9e37_79b9_7f4a_7c15;
        for n in [0, 1, 2, 3, 7, 8, 9, 31, 32, 33, 100, 1000] {
            let xs: Vec<(f64, usize)> = numbers(&mut seed, n, false).into_iter().zip(0..).collect();
            let (mut ours, mut std) = (xs.clone(), xs);
            sort_by(&mut ours, |a, b| b.0.total_cmp(&a.0));
            std.sort_by(|a, b| b.0.total_cmp(&a.0));
            assert_eq!(ours, std, "stable, newest first, as sort_by");
        }
    }

    #[test]
    fn a_comparator_that_says_equal_for_nan_never_panics() {
        // As the JS's `(a, b) => b - a` ports compare: std's sort panics on
        // these within a few tries.
        let mut seed = 12345;
        for n in [5, 20, 21, 33, 64, 100, 1000] {
            for _ in 0..200 {
                let xs = numbers(&mut seed, n, true);
                let mut sorted = xs.clone();
                sort_by(&mut sorted, |a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal));
                // The same numbers, moved.
                let bits = |v: &[f64]| {
                    let mut b: Vec<u64> = v.iter().map(|x| x.to_bits()).collect();
                    b.sort_unstable();
                    b
                };
                assert_eq!(bits(&sorted), bits(&xs));
            }
        }
        let mut xs = vec![f64::NAN, 3.0, 1.0, f64::NAN, 2.0];
        sort_by(&mut xs, |a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        assert_eq!(xs.iter().filter(|x| x.is_nan()).count(), 2);
    }
}
