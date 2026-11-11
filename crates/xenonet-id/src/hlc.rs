//! A hybrid logical clock: wall-clock milliseconds with a counter, so
//! entries from several machines sort in an order that respects what each
//! machine saw before it wrote. It orders things for display only. Nothing
//! may drop, refuse or reorder delivery on an HLC: a machine's clock can be
//! wrong, and an entry from it still counts.
//!
//! An [`Hlc`] is 48 bits of Unix milliseconds and a 16-bit counter, packed
//! into a u64 as `ms << 16 | counter`, so the u64's order is the clock's.
//! Its text is those 64 bits as 16 lowercase hex digits, which sort the same
//! way as strings and stay exact in JSON, where a number past 2^53 doesn't.
//!
//! A [`Clock`] hands out this machine's HLCs ([`Clock::now`]) and takes
//! other machines' ([`Clock::receive`]). A remote value more than
//! [`MAX_AHEAD_MS`] ahead of the local clock is clamped to that bound before
//! the clock takes it, so one machine with a clock far in the future can't
//! push everyone's HLCs there for good. The skew is reported instead, for
//! the doctor to show per peer.

use std::fmt;

use crate::{IdError, R};

/// The largest milliseconds an HLC holds: 2^48 - 1, about the year 10889.
pub const MAX_MS: u64 = (1 << 48) - 1;

/// How far ahead of the local clock a remote HLC may move it: 5 minutes.
pub const MAX_AHEAD_MS: u64 = 5 * 60 * 1000;

/// One reading of a hybrid logical clock: milliseconds and a counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Hlc(u64);

impl Hlc {
    /// The earliest HLC, below any a clock hands out.
    pub const ZERO: Hlc = Hlc(0);
    /// The latest HLC. A clock that reaches it stays there.
    pub const MAX: Hlc = Hlc(u64::MAX);

    /// The HLC at `ms` with `counter`. Milliseconds past [`MAX_MS`] are
    /// taken as [`MAX_MS`].
    pub const fn new(ms: u64, counter: u16) -> Hlc {
        let ms = if ms > MAX_MS { MAX_MS } else { ms };
        Hlc((ms << 16) | counter as u64)
    }

    /// The HLC packed in `bits`. Every u64 is one.
    pub const fn from_u64(bits: u64) -> Hlc {
        Hlc(bits)
    }

    /// The packed form, `ms << 16 | counter`.
    pub const fn to_u64(self) -> u64 {
        self.0
    }

    /// Unix milliseconds.
    pub const fn ms(self) -> u64 {
        self.0 >> 16
    }

    /// The counter within its millisecond.
    pub const fn counter(self) -> u16 {
        (self.0 & 0xffff) as u16
    }

    /// The next HLC up. A full counter carries into the milliseconds; at
    /// [`Hlc::MAX`] it stays.
    #[must_use]
    pub const fn next(self) -> Hlc {
        Hlc(self.0.saturating_add(1))
    }

    /// The canonical text: 16 lowercase hex digits.
    pub fn encode(self) -> String {
        format!("{:016x}", self.0)
    }

    /// The HLC in `text`, which must be exactly what [`Hlc::encode`] writes,
    /// so each HLC has one text.
    pub fn decode(text: &str) -> R<Hlc> {
        let canonical = text.len() == 16 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !canonical {
            return Err(IdError::refused("an HLC is 16 lowercase hex digits"));
        }
        u64::from_str_radix(text, 16).map(Hlc).map_err(|_| IdError::refused("an HLC is 16 lowercase hex digits"))
    }
}

impl fmt::Display for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// What taking a remote HLC did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Received {
    /// The local clock after taking it, as [`Clock::last`] now reads.
    pub local: Hlc,
    /// The remote HLC as the clock took it, and as it is shown: the value
    /// itself, or [`MAX_AHEAD_MS`] past the local milliseconds if it was
    /// further ahead.
    pub shown: Hlc,
    /// Whether the remote HLC was further ahead than [`MAX_AHEAD_MS`].
    pub clamped: bool,
    /// The remote milliseconds less the local wall clock's: positive when
    /// the remote is ahead, negative when it's behind. For the doctor.
    pub skew_ms: i64,
}

/// This machine's hybrid logical clock. Every reading it hands out is above
/// the one before, whatever the wall clock does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Clock {
    last: Hlc,
}

impl Clock {
    /// A clock that has handed out nothing yet.
    pub const fn new() -> Clock {
        Clock { last: Hlc::ZERO }
    }

    /// A clock that carries on from `last`, the latest reading it handed
    /// out or took, as kept on disk.
    pub const fn resume(last: Hlc) -> Clock {
        Clock { last }
    }

    /// The latest reading the clock handed out or took.
    pub const fn last(&self) -> Hlc {
        self.last
    }

    /// A reading for something written here, with the wall clock at
    /// `local_ms`: that millisecond with counter 0, or the next HLC after
    /// the last one, whichever is later.
    pub fn now(&mut self, local_ms: u64) -> Hlc {
        self.last = self.last.next().max(Hlc::new(local_ms, 0));
        self.last
    }

    /// Takes `remote`, an HLC another machine wrote, with the wall clock at
    /// `local_ms`. The clock moves past the later of its last reading, the
    /// wall clock and the remote value, clamped to [`MAX_AHEAD_MS`] past
    /// `local_ms`.
    pub fn receive(&mut self, remote: Hlc, local_ms: u64) -> Received {
        let local_ms = local_ms.min(MAX_MS);
        let bound = Hlc::new(local_ms.saturating_add(MAX_AHEAD_MS), u16::MAX);
        let shown = remote.min(bound);
        self.last = self.last.next().max(shown.next()).max(Hlc::new(local_ms, 0));
        Received { local: self.last, shown, clamped: remote > bound, skew_ms: skew(remote.ms(), local_ms) }
    }
}

// remote - local in i64; both are under 2^48, so it can't overflow.
fn skew(remote_ms: u64, local_ms: u64) -> i64 {
    remote_ms as i64 - local_ms as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // 2026-10-20T16:00:00Z, about where the clocks are.
    const T: u64 = 1_792_512_000_000;

    #[test]
    fn known_answers() {
        // The packed form and its text, as other implementations must write
        // them.
        for (ms, counter, bits, text) in [
            (0, 0, 0, "0000000000000000"),
            (T, 0, 0x01a1_5a0a_8000_0000, "01a15a0a80000000"),
            (T, 7, 0x01a1_5a0a_8000_0007, "01a15a0a80000007"),
            (T + 1, 0xffff, 0x01a1_5a0a_8001_ffff, "01a15a0a8001ffff"),
            (MAX_MS, 0xffff, u64::MAX, "ffffffffffffffff"),
        ] {
            let h = Hlc::new(ms, counter);
            assert_eq!(h.to_u64(), bits, "{ms} {counter}");
            assert_eq!(h.encode(), text);
            assert_eq!(h.to_string(), text);
            assert_eq!(Hlc::decode(text).unwrap(), h);
        }
    }

    #[test]
    fn decode_takes_only_the_canonical_text() {
        for bad in ["", "01a15a0a8000000", "01a15a0a800000000", "01A15A0A80000000", "+1a15a0a80000000", " 1a15a0a80000000", "01a15a0a8000000g"] {
            assert!(Hlc::decode(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn milliseconds_past_the_field_are_the_largest() {
        assert_eq!(Hlc::new(u64::MAX, 3), Hlc::new(MAX_MS, 3));
    }

    #[test]
    fn a_full_counter_carries_into_the_milliseconds() {
        let mut c = Clock::resume(Hlc::new(T, u16::MAX));
        assert_eq!(c.now(T), Hlc::new(T + 1, 0));
        assert_eq!(c.now(T), Hlc::new(T + 1, 1));
        let mut top = Clock::resume(Hlc::MAX);
        assert_eq!(top.now(T), Hlc::MAX, "stays at the top rather than wrapping");
        assert_eq!(top.receive(Hlc::MAX, u64::MAX).local, Hlc::MAX);
    }

    #[test]
    fn now_follows_the_wall_clock_and_counts_within_a_millisecond() {
        let mut c = Clock::new();
        assert_eq!(c.now(T), Hlc::new(T, 0));
        assert_eq!(c.now(T), Hlc::new(T, 1));
        assert_eq!(c.now(T - 1000), Hlc::new(T, 2), "a wall clock that went back");
        assert_eq!(c.now(T + 5), Hlc::new(T + 5, 0));
    }

    #[test]
    fn receive_takes_a_later_remote_and_clamps_a_far_one() {
        let mut c = Clock::new();
        let r = c.receive(Hlc::new(T + 60_000, 4), T);
        assert_eq!(r, Received { local: Hlc::new(T + 60_000, 5), shown: Hlc::new(T + 60_000, 4), clamped: false, skew_ms: 60_000 });

        let mut c = Clock::new();
        let r = c.receive(Hlc::new(T + 6 * 60_000, 0), T);
        let bound = Hlc::new(T + MAX_AHEAD_MS, u16::MAX);
        assert_eq!(r, Received { local: bound.next(), shown: bound, clamped: true, skew_ms: 6 * 60_000 });
        assert_eq!(c.last().ms(), T + MAX_AHEAD_MS + 1);

        let mut c = Clock::new();
        let r = c.receive(Hlc::new(T - 2000, 9), T);
        assert_eq!(r, Received { local: Hlc::new(T, 0), shown: Hlc::new(T - 2000, 9), clamped: false, skew_ms: -2000 });
    }

    fn hlc() -> impl Strategy<Value = Hlc> {
        prop_oneof![any::<u64>().prop_map(Hlc::from_u64), (T - 600_000..T + 600_000, any::<u16>()).prop_map(|(ms, c)| Hlc::new(ms, c))]
    }

    // A step: the wall clock moves by a delta (back too), and the clock
    // writes or takes a remote value near or far from it.
    #[derive(Debug, Clone)]
    enum Step {
        Now(i64),
        Receive(i64, Hlc),
    }

    fn step() -> impl Strategy<Value = Step> {
        prop_oneof![(-5000i64..5000).prop_map(Step::Now), (-5000i64..5000, hlc()).prop_map(|(d, h)| Step::Receive(d, h))]
    }

    proptest! {
        #[test]
        fn round_trip(bits in any::<u64>(), ms in any::<u64>(), counter in any::<u16>()) {
            let h = Hlc::from_u64(bits);
            prop_assert_eq!(Hlc::decode(&h.encode()).unwrap(), h);
            prop_assert_eq!(Hlc::new(h.ms(), h.counter()), h);
            let p = Hlc::new(ms, counter);
            prop_assert_eq!((p.ms(), p.counter()), (ms.min(MAX_MS), counter));
        }

        #[test]
        fn text_sorts_as_the_clock(a in any::<u64>(), b in any::<u64>()) {
            let (a, b) = (Hlc::from_u64(a), Hlc::from_u64(b));
            prop_assert_eq!(a.cmp(&b), a.encode().cmp(&b.encode()));
        }

        #[test]
        fn monotone_and_bounded(start in hlc(), steps in proptest::collection::vec(step(), 1..60)) {
            let mut c = Clock::resume(start);
            let mut wall = T;
            for s in steps {
                let before = c.last();
                match s {
                    Step::Now(d) => {
                        wall = wall.saturating_add_signed(d);
                        let h = c.now(wall);
                        prop_assert_eq!(c.last(), h);
                        prop_assert!(h > before || before == Hlc::MAX, "now moves forward");
                        prop_assert!(h >= Hlc::new(wall, 0));
                    }
                    Step::Receive(d, remote) => {
                        wall = wall.saturating_add_signed(d);
                        let r = c.receive(remote, wall);
                        prop_assert_eq!(c.last(), r.local);
                        let bound = Hlc::new(wall + MAX_AHEAD_MS, u16::MAX);
                        prop_assert!(r.local > before || before == Hlc::MAX, "receive moves forward");
                        prop_assert!(r.local >= before.max(r.shown).max(Hlc::new(wall, 0)), "receive is at least max(local, clamped remote)");
                        prop_assert!(r.shown <= bound, "the clamp holds");
                        prop_assert_eq!(r.clamped, remote > bound);
                        prop_assert_eq!(r.shown, if r.clamped { bound } else { remote });
                        // Only what the clock held before can be further ahead.
                        prop_assert!(r.local <= before.next().max(bound.next()));
                        prop_assert_eq!(r.skew_ms, remote.ms() as i64 - wall as i64);
                    }
                }
            }
        }
    }
}

// Model checking with Kani (`cargo kani -p xenonet-id`): the packing and
// the carry hold for every u64, not just the ones a proptest samples, and
// nothing here can panic.
#[cfg(kani)]
mod proofs {
    use super::*;

    #[kani::proof]
    fn packing_round_trips() {
        let ms: u64 = kani::any();
        let counter: u16 = kani::any();
        let h = Hlc::new(ms, counter);
        assert_eq!(h.ms(), ms.min(MAX_MS));
        assert_eq!(h.counter(), counter);
        let bits: u64 = kani::any();
        let p = Hlc::from_u64(bits);
        assert_eq!(Hlc::new(p.ms(), p.counter()), p);
    }

    #[kani::proof]
    fn next_carries_and_never_wraps() {
        let h = Hlc::from_u64(kani::any());
        let n = h.next();
        if h == Hlc::MAX {
            assert_eq!(n, Hlc::MAX);
        } else {
            assert!(n > h);
            if h.counter() == u16::MAX {
                assert_eq!((n.ms(), n.counter()), (h.ms() + 1, 0));
            } else {
                assert_eq!((n.ms(), n.counter()), (h.ms(), h.counter() + 1));
            }
        }
    }

    #[kani::proof]
    fn now_and_receive_are_monotone_and_clamped() {
        let last = Hlc::from_u64(kani::any());
        let local_ms: u64 = kani::any();
        let mut c = Clock::resume(last);
        let h = c.now(local_ms);
        assert!(h > last || last == Hlc::MAX);

        let mut c = Clock::resume(last);
        let remote = Hlc::from_u64(kani::any());
        let r = c.receive(remote, local_ms);
        let bound = Hlc::new(local_ms.min(MAX_MS).saturating_add(MAX_AHEAD_MS), u16::MAX);
        assert!(r.local > last || last == Hlc::MAX);
        assert!(r.local >= r.shown);
        assert!(r.shown <= bound);
        assert_eq!(r.clamped, remote > bound);
    }
}
