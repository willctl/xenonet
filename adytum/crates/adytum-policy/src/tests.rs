use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

use adytum_model::{Arch, CellName, CellSpec, DriverKind, ImagePin, Mount, NetworkName, Port, RefusalCode, Resources, SecretRef};
use proptest::prelude::*;

use crate::*;

fn name(s: &str) -> CellName {
    s.parse().unwrap()
}

fn spec(n: &str, wall: &str) -> CellSpec {
    CellSpec {
        name: name(n),
        driver: DriverKind::Lima,
        image: ImagePin::new("https://example.test/a.img".into(), format!("sha256:{}", "0".repeat(64)), Arch::Aarch64).unwrap(),
        wall: wall.parse().unwrap(),
        resources: Resources::default(),
        ports: vec![],
        automation_port: None,
        mounts: vec![],
        sealed: false,
        network: None,
        clone_from: None,
        secrets: vec![],
    }
}

fn facts(n: &str, wall: &str) -> CellFacts {
    CellFacts { name: name(n), wall: wall.parse().unwrap(), network: None, mount_sources: vec![], secrets: vec![], holds_open_eidolon: false }
}

fn port(guest: u16, bind: IpAddr) -> Port {
    Port { guest, host: guest + 10000, bind }
}

fn mount(src: &str) -> Mount {
    Mount { source: PathBuf::from(src), target: "/mnt/x".into(), writable: false }
}

fn world(cells: Vec<CellFacts>) -> World {
    World { cells }
}

fn code<T>(r: Result<T, adytum_model::Refusal>) -> RefusalCode {
    r.err().expect("should be refused").code
}

#[test]
fn ports_on_other_addresses_are_refused() {
    let mut s = spec("adytum-a", "wall_a");
    s.ports = vec![port(80, Ipv4Addr::UNSPECIFIED.into())];
    assert_eq!(code(ports_on_localhost(&s)), RefusalCode::PortNotLocalhost);
    s.ports = vec![port(80, Ipv4Addr::new(192, 168, 1, 5).into())];
    assert_eq!(code(ports_on_localhost(&s)), RefusalCode::PortNotLocalhost);
    s.ports = vec![port(80, Ipv6Addr::UNSPECIFIED.into())];
    assert_eq!(code(ports_on_localhost(&s)), RefusalCode::PortNotLocalhost);
}

#[test]
fn loopback_ports_pass() {
    let mut s = spec("adytum-a", "wall_a");
    s.ports = vec![port(80, Ipv4Addr::LOCALHOST.into()), port(81, Ipv4Addr::new(127, 5, 5, 5).into()), port(82, Ipv6Addr::LOCALHOST.into())];
    assert!(ports_on_localhost(&s).is_ok());
}

#[test]
fn published_automation_port_is_refused_even_on_localhost() {
    let mut s = spec("adytum-a", "wall_a");
    s.automation_port = Some(9222);
    s.ports = vec![port(9222, Ipv4Addr::LOCALHOST.into())];
    assert_eq!(code(automation_port_kept_inside(&s)), RefusalCode::AutomationPortPublished);
}

#[test]
fn automation_port_not_in_ports_passes() {
    let mut s = spec("adytum-a", "wall_a");
    s.automation_port = Some(9222);
    s.ports = vec![port(80, Ipv4Addr::LOCALHOST.into())];
    assert!(automation_port_kept_inside(&s).is_ok());
}

#[test]
fn sealed_cell_with_a_mount_is_refused() {
    let mut s = spec("adytum-a", "wall_a");
    s.sealed = true;
    s.mounts = vec![mount("/work/a")];
    assert_eq!(code(sealed_has_no_mounts(&s)), RefusalCode::SealedCellMount);
}

#[test]
fn mounts_are_fine_unless_sealed() {
    let mut s = spec("adytum-a", "wall_a");
    s.mounts = vec![mount("/work/a")];
    assert!(sealed_has_no_mounts(&s).is_ok());
    s.mounts.clear();
    s.sealed = true;
    assert!(sealed_has_no_mounts(&s).is_ok());
}

#[test]
fn names_outside_adytum_are_refused() {
    for n in ["pane", "xenonet-vm1", "default"] {
        assert_eq!(code(name_is_ours(n)), RefusalCode::NameNotOurs, "{n}");
    }
    assert_eq!(code(name_is_ours("adytum-")), RefusalCode::NameNotOurs);
    assert_eq!(code(name_is_ours("adytum-A")), RefusalCode::NameNotOurs);
}

#[test]
fn our_names_pass() {
    assert!(name_is_ours("adytum-test-1").is_ok());
}

#[test]
fn a_taken_name_is_refused() {
    let s = spec("adytum-a", "wall_a");
    let w = world(vec![facts("adytum-a", "wall_a")]);
    assert_eq!(code(name_free(&s, &w)), RefusalCode::NameTaken);
}

#[test]
fn a_free_name_passes() {
    let s = spec("adytum-b", "wall_a");
    assert!(name_free(&s, &world(vec![facts("adytum-a", "wall_a")])).is_ok());
}

#[test]
fn other_walls_cannot_share_a_network() {
    let mut s = spec("adytum-b", "wall_b");
    s.network = Some("net-1".parse().unwrap());
    let mut theirs = facts("adytum-a", "wall_a");
    theirs.network = Some("net-1".parse().unwrap());
    assert_eq!(code(walls_share_no_network(&s, &world(vec![theirs]))), RefusalCode::WallNetwork);
}

#[test]
fn same_wall_may_share_a_network_and_other_networks_pass() {
    let mut s = spec("adytum-b", "wall_a");
    s.network = Some("net-1".parse().unwrap());
    let mut theirs = facts("adytum-a", "wall_a");
    theirs.network = Some("net-1".parse().unwrap());
    assert!(walls_share_no_network(&s, &world(vec![theirs.clone()])).is_ok());
    s.wall = "wall_b".parse().unwrap();
    theirs.network = Some("net-2".parse().unwrap());
    assert!(walls_share_no_network(&s, &world(vec![theirs])).is_ok());
}

#[test]
fn other_walls_cannot_share_a_mount_source() {
    let mut s = spec("adytum-b", "wall_b");
    s.mounts = vec![mount("/work/x")];
    let mut theirs = facts("adytum-a", "wall_a");
    theirs.mount_sources = vec![PathBuf::from("/work/x")];
    assert_eq!(code(walls_share_no_mount(&s, &world(vec![theirs]))), RefusalCode::WallMount);
}

#[test]
fn a_mount_inside_or_around_another_walls_mount_is_shared() {
    let mut theirs = facts("adytum-a", "wall_a");
    theirs.mount_sources = vec![PathBuf::from("/work/x")];
    let w = world(vec![theirs]);
    for src in ["/work/x/sub", "/work", "/work/x/../x/sub", "/work/./x"] {
        let mut s = spec("adytum-b", "wall_b");
        s.mounts = vec![mount(src)];
        assert_eq!(code(walls_share_no_mount(&s, &w)), RefusalCode::WallMount, "{src}");
    }
}

#[test]
fn unrelated_or_same_wall_mounts_pass() {
    let mut theirs = facts("adytum-a", "wall_a");
    theirs.mount_sources = vec![PathBuf::from("/work/x")];
    let w = world(vec![theirs]);
    let mut s = spec("adytum-b", "wall_b");
    for src in ["/work/xy", "/other", "/work/y/../z"] {
        s.mounts = vec![mount(src)];
        assert!(walls_share_no_mount(&s, &w).is_ok(), "{src}");
    }
    s.wall = "wall_a".parse().unwrap();
    s.mounts = vec![mount("/work/x")];
    assert!(walls_share_no_mount(&s, &w).is_ok());
}

#[test]
fn cloning_across_walls_is_refused() {
    let mut s = spec("adytum-b", "wall_b");
    s.clone_from = Some(name("adytum-a"));
    let w = world(vec![facts("adytum-a", "wall_a")]);
    assert_eq!(code(walls_share_no_clone_source(&s, &w)), RefusalCode::WallClone);
}

#[test]
fn cloning_within_a_wall_passes() {
    let mut s = spec("adytum-b", "wall_a");
    s.clone_from = Some(name("adytum-a"));
    let w = world(vec![facts("adytum-a", "wall_a")]);
    assert!(walls_share_no_clone_source(&s, &w).is_ok());
}

#[test]
fn sharing_a_secret_across_walls_is_refused() {
    let mut s = spec("adytum-b", "wall_b");
    s.secrets = vec![SecretRef { name: "token".into() }];
    let mut theirs = facts("adytum-a", "wall_a");
    theirs.secrets = vec!["token".into()];
    assert_eq!(code(walls_share_no_secret(&s, &world(vec![theirs]))), RefusalCode::WallSecret);
}

#[test]
fn different_secrets_or_same_wall_pass() {
    let mut s = spec("adytum-b", "wall_b");
    s.secrets = vec![SecretRef { name: "token".into() }];
    let mut theirs = facts("adytum-a", "wall_a");
    theirs.secrets = vec!["other".into()];
    assert!(walls_share_no_secret(&s, &world(vec![theirs.clone()])).is_ok());
    theirs.secrets = vec!["token".into()];
    s.wall = "wall_a".parse().unwrap();
    assert!(walls_share_no_secret(&s, &world(vec![theirs])).is_ok());
}

#[test]
fn cloning_a_cell_with_an_open_eidolon_is_refused() {
    let mut s = spec("adytum-b", "wall_a");
    s.clone_from = Some(name("adytum-a"));
    let mut theirs = facts("adytum-a", "wall_a");
    theirs.holds_open_eidolon = true;
    assert_eq!(code(no_clone_of_open_eidolon(&s, &world(vec![theirs]))), RefusalCode::CloneOpenEidolon);
}

#[test]
fn cloning_a_cell_without_one_passes() {
    let mut s = spec("adytum-b", "wall_a");
    s.clone_from = Some(name("adytum-a"));
    assert!(no_clone_of_open_eidolon(&s, &world(vec![facts("adytum-a", "wall_a")])).is_ok());
    s.clone_from = None;
    assert!(no_clone_of_open_eidolon(&s, &world(vec![])).is_ok());
}

#[test]
fn check_create_passes_a_clean_spec_and_reports_the_first_refusal() {
    let mut s = spec("adytum-b", "wall_a");
    assert!(check_create(&s, &World::default()).is_ok());
    // Both the port rule and the name rule would refuse: the port rule comes first.
    s.ports = vec![port(80, Ipv4Addr::UNSPECIFIED.into())];
    let w = world(vec![facts("adytum-b", "wall_a")]);
    assert_eq!(code(check_create(&s, &w)), RefusalCode::PortNotLocalhost);
    s.ports.clear();
    assert_eq!(code(check_create(&s, &w)), RefusalCode::NameTaken);
}

proptest! {
    #[test]
    fn any_non_loopback_bind_is_refused(octets in any::<[u8; 4]>(), v6 in any::<[u16; 8]>(), use_v6 in any::<bool>()) {
        let ip: IpAddr = if use_v6 { Ipv6Addr::from(v6).into() } else { Ipv4Addr::from(octets).into() };
        prop_assume!(!ip.is_loopback());
        let mut s = spec("adytum-a", "wall_a");
        s.ports = vec![port(80, ip)];
        prop_assert_eq!(code(ports_on_localhost(&s)), RefusalCode::PortNotLocalhost);
        prop_assert_eq!(code(check_create(&s, &World::default())), RefusalCode::PortNotLocalhost);
    }

    #[test]
    fn different_walls_with_a_common_network_are_refused(
        a in "[a-z0-9]{1,20}",
        b in "[a-z0-9]{1,20}",
        net in "[a-z0-9-]{1,48}",
    ) {
        prop_assume!(a != b);
        let mut s = spec("adytum-new", &format!("wall_{a}"));
        let net: NetworkName = net.parse().unwrap();
        s.network = Some(net.clone());
        let mut theirs = facts("adytum-old", &format!("wall_{b}"));
        theirs.network = Some(net);
        prop_assert_eq!(
            code(check_create(&s, &world(vec![theirs]))),
            RefusalCode::WallNetwork
        );
    }

    #[test]
    fn a_valid_spec_into_an_empty_world_passes(
        suffix in "[a-z0-9][a-z0-9-]{0,45}[a-z0-9]",
        wall in "[a-z0-9_-]{1,64}",
        ports in proptest::collection::vec((1u16..9000, any::<u8>()), 0..4),
        automation in proptest::option::of(9000u16..9999),
        sealed in any::<bool>(),
    ) {
        let mut s = spec(&format!("adytum-{suffix}"), &format!("wall_{wall}"));
        s.ports = ports
            .into_iter()
            .map(|(g, last)| port(g, Ipv4Addr::new(127, 0, 0, last).into()))
            .collect();
        s.automation_port = automation;
        s.sealed = sealed;
        if !sealed {
            s.mounts = vec![mount("/work/a")];
        }
        prop_assert!(check_create(&s, &World::default()).is_ok());
    }
}
