//! Eidolon's rules as pure functions: one seat, the holder list, the wall,
//! what a facet may leave out. Each returns `Ok(())` or the `Refusal` that
//! says why not and what to do. Nothing here reads a file or a clock.

use eidolon_model::{verify, EidolonSpec, HolderKey, Refusal, RefusalCode, SignedSeat, State, Wall};

/// A machine off the holder list may not hold the eidolon at all.
pub fn may_hold(spec: &EidolonSpec, machine: &HolderKey) -> Result<(), Refusal> {
    if spec.holder_by_key(machine).is_some() {
        Ok(())
    } else {
        Err(not_a_holder())
    }
}

/// A machine may open the eidolon only when the current seat names it, the
/// eidolon is not retired and it is not open anywhere else.
///
/// `state` is where the eidolon is now (retired ones never open) and
/// `already_open_elsewhere` is what the caller found when it looked for a
/// cell holding it.
pub fn may_open(state: &State, current: &SignedSeat, machine: &HolderKey, already_open_elsewhere: bool) -> Result<(), Refusal> {
    if *state == State::Retired {
        return Err(Refusal::new(RefusalCode::Retired, "That eidolon is retired.", "Make a new eidolon; a retired one never opens again."));
    }
    if current.record.holder != *machine {
        return Err(Refusal::new(RefusalCode::SecondSeat, "The seat is another machine's.", "Move the eidolon here first, from the machine that holds it."));
    }
    if already_open_elsewhere || matches!(state, State::Open { .. }) {
        return Err(Refusal::new(RefusalCode::SecondSeat, "That eidolon is already open in a cell.", "Close it there before opening it again."));
    }
    Ok(())
}

/// Whether `next` may replace `current` as the seat. The first seat
/// (`current` is none) is generation 1, has no previous holder and is signed
/// by its own holder. Every later one is the next generation, signed by the
/// holder of the seat it replaces, naming a machine on the list, with that
/// holder as its `previous`.
pub fn accept_seat(spec: &EidolonSpec, current: Option<&SignedSeat>, next: &SignedSeat) -> Result<(), Refusal> {
    let n = &next.record;
    if n.eidolon != spec.id || current.is_some_and(|c| c.record.eidolon != n.eidolon) {
        return Err(Refusal::new(RefusalCode::StaleGeneration, "That seat record is for another eidolon.", "Ask the sender for this eidolon's seat."));
    }
    let want = current.map_or(1, |c| c.record.generation.saturating_add(1));
    if n.generation != want || (current.is_some() && want == 1) {
        return Err(Refusal::new(
            RefusalCode::StaleGeneration,
            format!("That seat is generation {} and this one needs {want}.", n.generation),
            "Ask the sender to start the move again from the current seat.",
        ));
    }
    let signer = current.map_or(&n.holder, |c| &c.record.holder);
    if !verify(next, signer) {
        return Err(Refusal::new(
            RefusalCode::SeatNotSignedByHolder,
            "That seat was not signed by the machine that holds the current one.",
            "Move the eidolon from the machine that holds it.",
        ));
    }
    may_hold(spec, &n.holder)?;
    if n.previous != current.map(|c| c.record.holder) {
        return Err(Refusal::new(
            RefusalCode::StaleGeneration,
            "That seat does not name the current holder as the previous one.",
            "Ask the sender to start the move again from the current seat.",
        ));
    }
    Ok(())
}

/// An eidolon opens only in a cell behind its own wall.
pub fn same_wall(eidolon_wall: &Wall, cell_wall: &Wall) -> Result<(), Refusal> {
    if eidolon_wall == cell_wall {
        Ok(())
    } else {
        Err(Refusal::new(RefusalCode::WrongWall, format!("That eidolon belongs behind {eidolon_wall}, not {cell_wall}."), "Open it in a cell behind its own wall."))
    }
}

/// A leave-out pattern stays inside its facet: relative, no `..`.
pub fn leave_out_is_inside(pattern: &str) -> Result<(), Refusal> {
    let escapes = pattern.starts_with('/') || pattern.starts_with('\\') || pattern.split(['/', '\\']).any(|part| part == "..") || pattern.contains('\0');
    if escapes {
        Err(Refusal::new(RefusalCode::LeaveOutEscapes, format!("The leave-out pattern {pattern:?} reaches outside its facet."), "Write it relative to the facet, without `..`."))
    } else {
        Ok(())
    }
}

fn not_a_holder() -> Refusal {
    Refusal::new(RefusalCode::NotAHolder, "That machine is not on the holder list.", "Add it to the eidolon's holders first, or pick a listed machine.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use eidolon_model::{EidolonId, Holder, SeatRecord};
    use proptest::prelude::*;
    use xenonet_id::sign::{KeyHome, SeedSigner, Signer};

    struct Machine {
        signer: SeedSigner,
        key: HolderKey,
    }

    fn machine(n: u8) -> Machine {
        let signer = SeedSigner::from_seed(&[n; 32], KeyHome::File).unwrap();
        let key = HolderKey::from_verifying_key(&signer.public()).unwrap();
        Machine { signer, key }
    }

    fn seat(eid: EidolonId, generation: u64, holder: &Machine, previous: Option<&Machine>, signed_by: &Machine) -> SignedSeat {
        let record = SeatRecord { eidolon: eid, generation, holder: holder.key, previous: previous.map(|p| p.key), at: "2026-01-02T03:04:05Z".into() };
        SignedSeat::sign(record, &signed_by.signer).unwrap()
    }

    fn spec_for(machines: &[&Machine]) -> EidolonSpec {
        let wall: Wall = "wall_work".parse().unwrap();
        let holders = machines.iter().enumerate().map(|(i, m)| Holder { name: format!("m{i}"), sign: m.key, seal: [0; 32] }).collect();
        EidolonSpec { id: EidolonId::from_bytes([5; 16]), wall, holders, facets: vec![] }
    }

    fn code(r: Result<(), Refusal>) -> Option<RefusalCode> {
        r.err().map(|e| e.code)
    }

    // may_hold

    #[test]
    fn a_machine_off_the_list_may_not_hold() {
        let (desk, tower, stranger) = (machine(1), machine(2), machine(3));
        let spec = spec_for(&[&desk, &tower]);
        assert_eq!(code(may_hold(&spec, &stranger.key)), Some(RefusalCode::NotAHolder));
    }

    #[test]
    fn a_listed_machine_may_hold() {
        let (desk, tower) = (machine(1), machine(2));
        let spec = spec_for(&[&desk, &tower]);
        assert!(may_hold(&spec, &tower.key).is_ok());
    }

    // may_open

    #[test]
    fn a_seat_naming_another_machine_does_not_open_here() {
        let (desk, tower) = (machine(1), machine(2));
        let s = seat(EidolonId::from_bytes([5; 16]), 1, &desk, None, &desk);
        assert_eq!(code(may_open(&State::Sealed, &s, &tower.key, false)), Some(RefusalCode::SecondSeat));
    }

    #[test]
    fn an_eidolon_open_elsewhere_does_not_open_again() {
        let desk = machine(1);
        let s = seat(EidolonId::from_bytes([5; 16]), 1, &desk, None, &desk);
        assert_eq!(code(may_open(&State::Sealed, &s, &desk.key, true)), Some(RefusalCode::SecondSeat));
        assert_eq!(code(may_open(&State::Open { cell: "adytum-a".into() }, &s, &desk.key, false)), Some(RefusalCode::SecondSeat));
    }

    #[test]
    fn a_retired_eidolon_never_opens() {
        let desk = machine(1);
        let s = seat(EidolonId::from_bytes([5; 16]), 1, &desk, None, &desk);
        assert_eq!(code(may_open(&State::Retired, &s, &desk.key, false)), Some(RefusalCode::Retired));
    }

    #[test]
    fn the_seat_holder_opens_a_sealed_eidolon() {
        let desk = machine(1);
        let s = seat(EidolonId::from_bytes([5; 16]), 1, &desk, None, &desk);
        assert!(may_open(&State::Sealed, &s, &desk.key, false).is_ok());
    }

    // accept_seat

    #[test]
    fn a_seat_that_skips_or_repeats_a_generation_is_refused() {
        let (desk, tower) = (machine(1), machine(2));
        let spec = spec_for(&[&desk, &tower]);
        let cur = seat(spec.id, 3, &desk, None, &desk);
        for g in [3, 5, 1] {
            let next = seat(spec.id, g, &tower, Some(&desk), &desk);
            assert_eq!(code(accept_seat(&spec, Some(&cur), &next)), Some(RefusalCode::StaleGeneration), "generation {g}");
        }
        let second_first = seat(spec.id, 2, &desk, None, &desk);
        assert_eq!(code(accept_seat(&spec, None, &second_first)), Some(RefusalCode::StaleGeneration), "a first seat is generation 1");
    }

    #[test]
    fn a_seat_for_another_eidolon_is_refused() {
        let (desk, tower) = (machine(1), machine(2));
        let spec = spec_for(&[&desk, &tower]);
        let other = seat(EidolonId::from_bytes([6; 16]), 1, &desk, None, &desk);
        assert_eq!(code(accept_seat(&spec, None, &other)), Some(RefusalCode::StaleGeneration));
    }

    #[test]
    fn a_seat_not_signed_by_the_current_holder_is_refused() {
        let (desk, tower, third) = (machine(1), machine(2), machine(3));
        let spec = spec_for(&[&desk, &tower, &third]);
        let cur = seat(spec.id, 1, &desk, None, &desk);
        // Signed by the receiver itself, and by a third listed machine.
        for signer in [&tower, &third] {
            let next = seat(spec.id, 2, &tower, Some(&desk), signer);
            assert_eq!(code(accept_seat(&spec, Some(&cur), &next)), Some(RefusalCode::SeatNotSignedByHolder));
        }
        // A first seat signed by someone other than its holder.
        let first = seat(spec.id, 1, &desk, None, &tower);
        assert_eq!(code(accept_seat(&spec, None, &first)), Some(RefusalCode::SeatNotSignedByHolder));
    }

    #[test]
    fn a_seat_naming_a_machine_off_the_list_is_refused() {
        let (desk, stranger) = (machine(1), machine(3));
        let spec = spec_for(&[&desk]);
        let cur = seat(spec.id, 1, &desk, None, &desk);
        let next = seat(spec.id, 2, &stranger, Some(&desk), &desk);
        assert_eq!(code(accept_seat(&spec, Some(&cur), &next)), Some(RefusalCode::NotAHolder));
    }

    #[test]
    fn a_seat_with_the_wrong_previous_holder_is_refused() {
        let (desk, tower, third) = (machine(1), machine(2), machine(3));
        let spec = spec_for(&[&desk, &tower, &third]);
        let cur = seat(spec.id, 1, &desk, None, &desk);
        for previous in [Some(&third), None] {
            let next = seat(spec.id, 2, &tower, previous, &desk);
            assert_eq!(code(accept_seat(&spec, Some(&cur), &next)), Some(RefusalCode::StaleGeneration));
        }
    }

    #[test]
    fn the_first_seat_and_the_next_one_are_accepted() {
        let (desk, tower) = (machine(1), machine(2));
        let spec = spec_for(&[&desk, &tower]);
        let first = seat(spec.id, 1, &desk, None, &desk);
        assert!(accept_seat(&spec, None, &first).is_ok());
        let next = seat(spec.id, 2, &tower, Some(&desk), &desk);
        assert!(accept_seat(&spec, Some(&first), &next).is_ok());
    }

    // same_wall

    #[test]
    fn another_wall_is_refused_and_the_same_one_is_not() {
        let (a, b): (Wall, Wall) = ("wall_a".parse().unwrap(), "wall_b".parse().unwrap());
        assert_eq!(code(same_wall(&a, &b)), Some(RefusalCode::WrongWall));
        assert!(same_wall(&a, &a).is_ok());
    }

    // leave_out_is_inside

    #[test]
    fn a_leave_out_that_escapes_is_refused() {
        for p in ["/etc/passwd", "../x", "a/../../b", "a/..", "..", "\\windows", "a\\..\\b", "a\0b"] {
            assert_eq!(code(leave_out_is_inside(p)), Some(RefusalCode::LeaveOutEscapes), "{p:?}");
        }
    }

    #[test]
    fn a_relative_leave_out_is_fine() {
        for p in ["cache2/**", "*.lock", "a/b..c", "..hidden/x", "Crash Reports/**"] {
            assert!(leave_out_is_inside(p).is_ok(), "{p:?}");
        }
    }

    // properties

    proptest! {
        #[test]
        fn any_generation_but_the_next_is_refused(cur_gen in 1u64..u64::MAX - 1, next_gen in any::<u64>()) {
            prop_assume!(next_gen != cur_gen + 1);
            let (desk, tower) = (machine(1), machine(2));
            let spec = spec_for(&[&desk, &tower]);
            let cur = seat(spec.id, cur_gen, &desk, None, &desk);
            let next = seat(spec.id, next_gen, &tower, Some(&desk), &desk);
            prop_assert_eq!(code(accept_seat(&spec, Some(&cur), &next)), Some(RefusalCode::StaleGeneration));
        }

        #[test]
        fn any_machine_off_the_list_is_refused(key in any::<[u8; 32]>()) {
            let (desk, tower) = (machine(1), machine(2));
            let spec = spec_for(&[&desk, &tower]);
            let key = HolderKey::from_bytes(key);
            prop_assume!(key != desk.key && key != tower.key);
            prop_assert_eq!(code(may_hold(&spec, &key)), Some(RefusalCode::NotAHolder));
        }
    }
}
