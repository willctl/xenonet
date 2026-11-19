//! A network reaches only what it is shown: a frame from another machine
//! that reaches a session, a project or a handoff's target is judged on
//! whether the network it came in is shown that project, whoever sent it.

use crate::common::*;
use xenonet_authz::*;

fn atlas() -> ProjectId {
    ProjectId::new("prj_atlas").unwrap()
}
fn ledger() -> ProjectId {
    ProjectId::new("prj_ledger").unwrap()
}
fn session(s: &str) -> SessionId {
    SessionId::new(s).unwrap()
}

/// m2, a plain member of n1, which is shown only atlas. This machine has a
/// session in atlas, one in ledger and one in no project.
fn world() -> World {
    let mut w = fixture().world;
    w.shared = Some(vec![atlas()]);
    w.session_projects = [(session("s-atlas"), atlas()), (session("s-ledger"), ledger())].into();
    w
}

fn from_m2(w: &World, who: &str, resource: Resource, act: Act, reaching: &[Reaching]) -> Decision {
    let principal = match who {
        "agent" => Principal::Agent { id: AgentId::new("a2").unwrap(), machine: machine("m2"), attestation: Attestation::ProcessChecked },
        _ => Principal::Machine { id: machine("m2") },
    };
    let mut r = Request::new(principal, resource, act, Door::Network, NOW, Some(network()));
    for x in reaching {
        r = r.reaching(x.clone());
    }
    decide(None, &r, w, None)
}

fn message(w: &World, who: &str, session_id: &str) -> Decision {
    from_m2(w, who, Resource::Machine(machine("m1")), Act::Message, &[Reaching::Session(session(session_id))])
}

fn refused_as_unseen(d: &Decision) {
    let reason = d.reason().expect("refused");
    assert_eq!((reason.code(), reason.text()), ("shared", NOT_SEEN));
}

#[test]
fn a_message_reaches_a_session_only_in_a_project_the_network_is_shown() {
    let w = world();
    for who in ["machine", "agent"] {
        assert!(message(&w, who, "s-atlas").is_allow(), "{who}: into atlas");
        refused_as_unseen(&message(&w, who, "s-ledger"));
    }
    // A session in no confirmed project is placed nowhere: not shown to a
    // network that is shown projects, though one shown everything sees it.
    refused_as_unseen(&message(&w, "agent", "s-loose"));
    let mut all = world();
    all.shared = None;
    assert!(message(&all, "agent", "s-loose").is_allow() && message(&all, "agent", "s-ledger").is_allow());
}

#[test]
fn a_session_that_isnt_there_is_refused_in_the_same_words_as_one_that_isnt_shown() {
    let w = world();
    let missing = message(&w, "agent", "s-nowhere");
    let hidden = message(&w, "agent", "s-ledger");
    assert_eq!(missing.reason().map(|r| (r.code(), r.text())), hidden.reason().map(|r| (r.code(), r.text())));
    assert_eq!(hidden.reason().unwrap().text(), NOT_SEEN);
}

#[test]
fn a_wake_asks_the_same_question_whatever_the_wake_setting() {
    let mut w = world();
    w.settings = Settings { start: StartSetting::You, wake: WakeSetting::Network };
    let wake = |w: &World, s: &str| from_m2(w, "agent", Resource::Session(session(s)), Act::Wake, &[]);
    assert!(wake(&w, "s-atlas").is_allow());
    refused_as_unseen(&wake(&w, "s-ledger"));
    // Consent for the session doesn't open it either.
    w.settings = Settings::default();
    w.consent = Consent { here: true, actors: true };
    refused_as_unseen(&wake(&w, "s-ledger"));
    assert!(wake(&w, "s-atlas").is_allow());
}

#[test]
fn a_claim_is_judged_on_its_project() {
    let w = world();
    let claim = |p: &ProjectId| from_m2(&w, "agent", Resource::Project(p.clone()), Act::Claim { project: p.clone() }, &[]);
    assert!(claim(&atlas()).is_allow());
    refused_as_unseen(&claim(&ledger()));
}

#[test]
fn a_handoff_reaches_only_the_projects_it_names_that_the_network_is_shown() {
    let w = world();
    let handoff = |w: &World, reaching: &[Reaching]| from_m2(w, "agent", Resource::Machine(machine("m1")), Act::Handoff, reaching);
    assert!(handoff(&w, &[Reaching::Project(atlas())]).is_allow());
    assert!(handoff(&w, &[Reaching::Project(atlas()), Reaching::Session(session("s-atlas"))]).is_allow());
    refused_as_unseen(&handoff(&w, &[Reaching::Project(ledger())]));
    refused_as_unseen(&handoff(&w, &[Reaching::Session(session("s-ledger"))]));
    // One shown project doesn't carry a hidden one along.
    refused_as_unseen(&handoff(&w, &[Reaching::Project(atlas()), Reaching::Session(session("s-ledger"))]));
    // One that names no project is for whatever takes it: only a network
    // shown everything reaches that far.
    refused_as_unseen(&handoff(&w, &[Reaching::Unplaced]));
    let mut all = world();
    all.shared = None;
    assert!(handoff(&all, &[Reaching::Unplaced]).is_allow());
    // A handoff that reaches nothing in particular is still a machine's.
    assert!(handoff(&w, &[]).is_allow());
}

#[test]
fn a_network_shown_nothing_reaches_nothing() {
    let mut w = world();
    w.shared = Some(Vec::new());
    refused_as_unseen(&message(&w, "agent", "s-atlas"));
    refused_as_unseen(&from_m2(&w, "agent", Resource::Project(atlas()), Act::Claim { project: atlas() }, &[]));
}

#[test]
fn this_machines_own_door_reaches_what_the_network_isnt_shown() {
    let w = world();
    let own = Principal::Agent { id: AgentId::new("a1").unwrap(), machine: machine("m1"), attestation: Attestation::ProcessChecked };
    let local = Request::new(own, Resource::Session(session("s-ledger")), Act::Message, Door::Local, NOW, None);
    assert!(decide(None, &local, &w, None).is_allow(), "what is shown to a network limits that network, not this machine");
}
