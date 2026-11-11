//! Tokens: round trips, roots and their certificates, revocation,
//! narrowing to the gateway, and page cursors.

use crate::common::*;
use xenonet_authz::*;
use xenonet_id::sign::Signer;

fn actor() -> Principal {
    Principal::Actor { slug: ActorSlug::new("muse").unwrap(), machine: machine("m1") }
}

fn message(token: &Token, world: &World, door: Door) -> Decision {
    let r = Request::new(actor(), Resource::Machine(machine("m1")), Act::Message, door, NOW, None);
    decide(Some(token), &r, world, None)
}

#[test]
fn a_token_round_trips_through_its_text() {
    let f = fixture();
    let mut grant = actor_grant("muse", &[Action::Wake, Action::Message, Action::Message]);
    grant.network = Some(network());
    let token = f.m1_root.mint_actor(&grant).unwrap();
    let text = token.encode(ACTOR_PREFIX).unwrap();
    assert!(text.starts_with("xenonet_actor_") && !text.contains('='), "base64url without padding");
    let back = Token::parse(&text, ACTOR_PREFIX, &f.world, NOW).unwrap();
    assert_eq!(back.bearer(), &Bearer { slug: ActorSlug::new("muse").unwrap(), machine: machine("m1") });
    assert_eq!(back.rights(), &[Action::Message, Action::Wake]);
    assert_eq!(back.network(), Some(&network()));
    assert_eq!(back.until(), NOW + HOUR);
    assert_eq!(back.root(), f.m1_root.certificate());
    assert_eq!(back.revocation_ids(), token.revocation_ids());
    assert_eq!(format!("{back:?}"), "[the token of the actor muse on m1]", "its bytes never show");
    // The other prefixes, as the caller chooses.
    let gateway = token.attenuate(&Narrowing::Door(Door::Gateway)).unwrap().encode(GATEWAY_PREFIX).unwrap();
    assert!(Token::parse(&gateway, GATEWAY_PREFIX, &f.world, NOW).is_ok());
    assert!(Token::parse(&gateway, ACTOR_PREFIX, &f.world, NOW).is_err(), "another prefix");
    assert!(token.encode("Bad Prefix").is_err());
}

#[test]
fn a_changed_or_cut_token_is_refused() {
    let f = fixture();
    let text = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Message])).unwrap().encode(ACTOR_PREFIX).unwrap();
    let mut flipped = text.clone().into_bytes();
    let i = flipped.len() - 10;
    flipped[i] = if flipped[i] == b'A' { b'B' } else { b'A' };
    assert!(Token::parse(std::str::from_utf8(&flipped).unwrap(), ACTOR_PREFIX, &f.world, NOW).is_err());
    assert!(Token::parse(&text[..text.len() - 8], ACTOR_PREFIX, &f.world, NOW).is_err());
    assert!(Token::parse("xenonet_actor_!!", ACTOR_PREFIX, &f.world, NOW).is_err());
    assert!(Token::parse(&format!("xenonet_actor_{}", "A".repeat(20_000)), ACTOR_PREFIX, &f.world, NOW).is_err());
}

#[test]
fn a_root_must_be_held_certified_and_in_its_time() {
    let f = fixture();
    let text = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Message])).unwrap().encode(ACTOR_PREFIX).unwrap();
    // Unknown: this machine doesn't hold the root.
    let mut w = fixture().world;
    w.roots.clear();
    assert_eq!(Token::parse(&text, ACTOR_PREFIX, &w, NOW).unwrap_err().code, "root");
    // Its machine's key isn't known.
    let mut w = fixture().world;
    w.roots = f.world.roots.clone();
    w.keys.remove(&machine("m1"));
    assert_eq!(Token::parse(&text, ACTOR_PREFIX, &w, NOW).unwrap_err().code, "root");
    // Before and after its time.
    assert_eq!(Token::parse(&text, ACTOR_PREFIX, &f.world, NOW - 2 * HOUR).unwrap_err().code, "root");
    assert_eq!(Token::parse(&text, ACTOR_PREFIX, &f.world, NOW + 2 * YEAR).unwrap_err().code, "root");
    // The token itself expired, within its root's time.
    assert_eq!(Token::parse(&text, ACTOR_PREFIX, &f.world, NOW + 2 * HOUR).unwrap_err().code, "expired");
}

#[test]
fn a_root_certificate_checks_against_its_machines_key_only() {
    let root = TokenRoot::new(machine("m1"), &m1_key(), NOW, NOW + HOUR).unwrap();
    let cert = root.certificate();
    cert.verify(&m1_key().public(), NOW).unwrap();
    assert!(cert.verify(&m2_key().public(), NOW).is_err(), "another machine's key");
    assert!(cert.verify(&m1_key().public(), NOW + 2 * HOUR).is_err(), "expired");
    assert!(cert.verify(&m1_key().public(), NOW - 1).is_err(), "not yet");
    assert_eq!(RootCertificate::parse(&cert.to_json()).as_ref(), Some(cert));
    // A certificate that m1's key signed claiming to be m2's root doesn't
    // check against m2's key, so tokens under it are refused.
    let forged = TokenRoot::new(machine("m2"), &m1_key(), NOW - HOUR, NOW + HOUR).unwrap();
    assert!(forged.certificate().verify(&m2_key().public(), NOW).is_err());
    let mut w = fixture().world;
    w.roots.push(forged.certificate().clone());
    let text = forged.mint_actor(&actor_grant("muse", &[Action::Message])).unwrap().encode(ACTOR_PREFIX).unwrap();
    assert_eq!(Token::parse(&text, ACTOR_PREFIX, &w, NOW).unwrap_err().code, "root");
    // A changed time breaks the signature.
    let mut v = cert.to_json();
    v["until"] = serde_json::json!(NOW + YEAR);
    assert!(RootCertificate::parse(&v).unwrap().verify(&m1_key().public(), NOW).is_err());
    // A token can't outlive its root.
    assert!(root.mint_actor(&ActorGrant { until: NOW + 2 * HOUR, ..actor_grant("muse", &[]) }).is_err());
}

#[test]
fn another_machines_token_speaks_only_for_its_actors() {
    let f = fixture();
    let theirs = f.m2_root.mint_actor(&actor_grant("muse", &[Action::Message])).unwrap();
    let parsed = Token::parse(&theirs.encode(ACTOR_PREFIX).unwrap(), ACTOR_PREFIX, &f.world, NOW).unwrap();
    // m2's token for its muse, used as m1's muse: refused.
    let r = Request::new(actor(), Resource::Machine(machine("m1")), Act::Message, Door::Local, NOW, None);
    assert_eq!(decide(Some(&parsed), &r, &f.world, None).reason().unwrap().code(), "token");
    // As m2's muse, from the network: allowed.
    let theirs_actor = Principal::Actor { slug: ActorSlug::new("muse").unwrap(), machine: machine("m2") };
    let r = Request::new(theirs_actor, Resource::Machine(machine("m1")), Act::Message, Door::Network, NOW, Some(network()));
    assert!(decide(Some(&parsed), &r, &f.world, None).is_allow());
}

#[test]
fn a_revoked_token_and_everything_narrowed_from_it_is_refused() {
    let mut f = fixture();
    let token = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Message])).unwrap();
    let child = token.attenuate(&Narrowing::Until(NOW + 60_000)).unwrap();
    assert!(message(&child, &f.world, Door::Local).is_allow());
    f.world.revoked.insert(token.revocation_ids()[0].clone());
    assert_eq!(Token::parse(&token.encode(ACTOR_PREFIX).unwrap(), ACTOR_PREFIX, &f.world, NOW).unwrap_err().code, "revoked");
    assert_eq!(Token::parse(&child.encode(ACTOR_PREFIX).unwrap(), ACTOR_PREFIX, &f.world, NOW).unwrap_err().code, "revoked");
    // A token parsed before the revocation arrived is refused once it's held.
    assert_eq!(message(&child, &f.world, Door::Local).reason().unwrap().code(), "revoked");
    assert_eq!(message(&token, &f.world, Door::Local).reason().unwrap().code(), "revoked");
}

#[test]
fn a_root_taken_out_of_the_ground_stops_its_tokens() {
    let mut f = fixture();
    let token = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Message])).unwrap();
    assert!(message(&token, &f.world, Door::Local).is_allow());
    f.world.roots.retain(|r| r != f.m1_root.certificate());
    assert_eq!(message(&token, &f.world, Door::Local).reason().unwrap().code(), "root");
}

#[test]
fn a_gateway_token_works_only_at_the_gateway_and_needs_its_right() {
    let f = fixture();
    let without = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Message])).unwrap();
    let with = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Message, Action::UseGateway])).unwrap();
    assert!(message(&without, &f.world, Door::Local).is_allow());
    let refused = message(&without, &f.world, Door::Gateway);
    assert_eq!(refused.reason().unwrap().text(), "the token has no right to message through the gateway");
    assert!(message(&with, &f.world, Door::Gateway).is_allow());
    assert!(message(&with, &f.world, Door::A2a).is_allow());
    let gateway = with.attenuate(&Narrowing::Door(Door::Gateway)).unwrap();
    assert!(message(&gateway, &f.world, Door::Gateway).is_allow());
    let at_home = message(&gateway, &f.world, Door::Local);
    assert!(at_home.reason().unwrap().text().starts_with("the token is narrowed: check if door(\"gateway\")"), "{at_home:?}");
    assert!(!message(&gateway, &f.world, Door::A2a).is_allow());
}

#[test]
fn narrowing_to_actions_and_times_takes_away_the_rest() {
    let f = fixture();
    let token = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Message, Action::Claim])).unwrap();
    let claim = |t: &Token, at| decide(Some(t), &Request::new(actor(), Resource::Project(project()), Act::Claim { project: project() }, Door::Local, at, None), &f.world, None);
    let only_messages = token.attenuate(&Narrowing::Actions(vec![Action::Message])).unwrap();
    assert!(claim(&token, NOW).is_allow());
    assert!(!claim(&only_messages, NOW).is_allow());
    assert!(message(&only_messages, &f.world, Door::Local).is_allow());
    let nothing = token.attenuate(&Narrowing::Actions(vec![])).unwrap();
    assert!(!message(&nothing, &f.world, Door::Local).is_allow());
    let soon = token.attenuate(&Narrowing::Until(NOW + 60_000)).unwrap();
    assert!(claim(&soon, NOW + 30_000).is_allow());
    assert!(!claim(&soon, NOW + 120_000).is_allow());
    let expired = claim(&token, NOW + 2 * HOUR);
    assert_eq!(expired.reason().unwrap().text(), "the token expired");
}

#[test]
fn an_actor_with_the_right_to_read_reads_and_a_cursor_pins_the_page() {
    let mut f = fixture();
    let token = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Read])).unwrap();
    let text = token.encode(ACTOR_PREFIX).unwrap();
    let token = Token::parse(&text, ACTOR_PREFIX, &f.world, NOW).unwrap();
    assert_eq!(token.bearer(), &Bearer { slug: ActorSlug::new("muse").unwrap(), machine: machine("m1") });
    assert_eq!(token.rights(), &[Action::Read]);
    let muse = Principal::Actor { slug: ActorSlug::new("muse").unwrap(), machine: machine("m1") };
    let read = |t: &Token, w: &World, scope, filters, at| {
        decide(Some(t), &Request::new(muse.clone(), Resource::Machine(machine("m1")), Act::Read { scope, filters }, Door::Local, at, None), w, None)
    };
    assert!(read(&token, &f.world, ReadScope::Search, None, NOW).is_allow());
    let msg = decide(Some(&token), &Request::new(muse.clone(), Resource::Machine(machine("m1")), Act::Message, Door::Local, NOW, None), &f.world, None);
    assert!(!msg.is_allow(), "it reads, and has no right to message");
    // The cursor: one op, one set of filters, five minutes.
    let filters = [7u8; 32];
    let cursor = token.cursor(ReadScope::Search, &filters, NOW, Some("WzUsImEiXQ")).unwrap();
    assert_eq!(cursor.page(), Some(("WzUsImEiXQ".to_string(), NOW + CURSOR_MS)), "where the page ended, as data");
    assert_eq!(token.page(), None, "the grant's own token is no cursor");
    assert!(token.cursor(ReadScope::Search, &filters, NOW, Some("not\"base64")).is_err());
    assert!(read(&cursor, &f.world, ReadScope::Search, Some(filters), NOW + 60_000).is_allow());
    assert!(!read(&cursor, &f.world, ReadScope::Search, Some([8; 32]), NOW + 60_000).is_allow(), "other filters");
    assert!(!read(&cursor, &f.world, ReadScope::Search, None, NOW + 60_000).is_allow(), "no filters");
    assert!(!read(&cursor, &f.world, ReadScope::Transcripts, Some(filters), NOW + 60_000).is_allow(), "another op");
    assert!(!read(&cursor, &f.world, ReadScope::Search, Some(filters), NOW + CURSOR_MS + 1000).is_allow(), "after five minutes");
    assert!(f.m1_root.mint_actor(&actor_grant("muse", &[Action::Message])).unwrap().cursor(ReadScope::Map, &filters, NOW, None).is_err(), "only a token that reads");
    // Revoking the grant stops it and every cursor made from it.
    f.world.revoked.insert(token.revocation_ids()[0].clone());
    assert_eq!(read(&token, &f.world, ReadScope::Search, None, NOW).reason().unwrap().code(), "revoked");
    assert_eq!(read(&cursor, &f.world, ReadScope::Search, Some(filters), NOW).reason().unwrap().code(), "revoked");
}

#[test]
fn a_tokens_ids_are_read_from_its_text_without_verifying_it() {
    let f = fixture();
    let token = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Message])).unwrap();
    let narrowed = token.attenuate(&Narrowing::Door(Door::Gateway)).unwrap();
    let text = narrowed.encode(GATEWAY_PREFIX).unwrap();
    let ids = Token::ids_of(&text, GATEWAY_PREFIX).unwrap();
    assert_eq!((ids.len(), ids[0].clone()), (2, token.id()), "the grant's id first, then the narrowing's");
    assert_eq!(Token::ids_of(&text, ACTOR_PREFIX), None);
    assert_eq!(Token::ids_of("xenonet_gateway_nope", GATEWAY_PREFIX), None);
}

// An actor's claims are checked against its grant's projects, which the
// receiver keeps (phase 4's grant_projects): a project its grant doesn't
// name, or a grant naming none, is refused.
#[test]
fn an_actor_claims_only_in_the_projects_its_grant_names() {
    let mut f = fixture();
    let token = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Claim])).unwrap();
    let claim = |w: &World, p: &str| {
        let p = ProjectId::new(p).unwrap();
        decide(Some(&token), &Request::new(actor(), Resource::Project(p.clone()), Act::Claim { project: p }, Door::Local, NOW, None), w, None)
    };
    assert!(claim(&f.world, "prj_atlas").is_allow(), "inside its grant");
    let outside = claim(&f.world, "prj_ledger");
    assert_eq!(outside.reason().map(Reason::code), Some("scope"), "outside its grant");
    f.world.projects = None;
    assert_eq!(claim(&f.world, "prj_atlas").reason().map(Reason::code), Some("scope"), "a grant that names no projects");
}
