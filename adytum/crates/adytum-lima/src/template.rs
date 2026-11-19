//! A cell spec as a Lima template.
//!
//! Lima's own defaults are not trusted: the home folder is mounted by
//! default, every port a guest listens on is forwarded, the host user's name
//! and login are copied into the guest, and the host's proxy variables are
//! passed on. The template turns each of those off, so what a cell can see
//! and reach is what the spec lists and nothing else.

use adytum_driver::DriverError;
use adytum_model::{Arch, CellSpec, DriverKind};

/// The guest account's name. The host user's name is never copied in.
const GUEST_USER: &str = "adytum";

fn quoted(text: &str) -> String {
    // A JSON string is also a YAML double-quoted string.
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned())
}

fn arch_name(arch: Arch) -> &'static str {
    match arch {
        Arch::Aarch64 => "aarch64",
        Arch::X86_64 => "x86_64",
    }
}

fn unsupported(what: &str) -> DriverError {
    DriverError::Unsupported { what: what.to_owned() }
}

/// The template for `spec`. The same spec always gives the same text.
///
/// Refuses what a Lima cell cannot honour (a clone, a network, secrets) and
/// re-checks the rules that guard the host, so a driver called without the
/// policy check first still cannot publish a port or mount a folder it should
/// not.
pub fn render(spec: &CellSpec) -> Result<String, DriverError> {
    adytum_policy::name_is_ours(spec.name.as_str())?;
    adytum_policy::ports_on_localhost(spec)?;
    adytum_policy::automation_port_kept_inside(spec)?;
    adytum_policy::sealed_has_no_mounts(spec)?;
    if spec.driver != DriverKind::Lima {
        return Err(unsupported("a cell for another driver"));
    }
    if spec.clone_from.is_some() {
        return Err(unsupported("cloning a cell"));
    }
    if spec.network.is_some() {
        return Err(unsupported("a named network"));
    }
    if !spec.secrets.is_empty() {
        return Err(unsupported("secrets"));
    }

    let mut out = String::new();
    let mut line = |text: &str| {
        out.push_str(text);
        out.push('\n');
    };
    line("# Written by adytum from a cell spec. Edit the spec, not this file.");
    line("minimumLimaVersion: 2.0.0");
    line("vmType: vz");
    line(&format!("arch: {}", arch_name(spec.image.arch)));
    line("images:");
    line(&format!("- location: {}", quoted(&spec.image.location)));
    line(&format!("  arch: {}", arch_name(spec.image.arch)));
    line(&format!("  digest: {}", quoted(&spec.image.digest)));
    line(&format!("cpus: {}", spec.resources.cpus));
    line(&format!("memory: {}MiB", spec.resources.memory_mib));
    line(&format!("disk: {}GiB", spec.resources.disk_gib));

    // A sealed cell always gets an empty list: left out, Lima mounts the
    // home folder.
    let mounts: &[_] = if spec.sealed { &[] } else { &spec.mounts };
    if mounts.is_empty() {
        line("mounts: []");
    } else {
        line("mounts:");
        for m in mounts {
            let source = m.source.to_string_lossy();
            if !m.source.is_absolute() || !m.target.starts_with('/') || source.contains("..") || m.target.contains("..") {
                return Err(unsupported("a mount whose folder or target is not an absolute path without .."));
            }
            line(&format!("- location: {}", quoted(&source)));
            line(&format!("  mountPoint: {}", quoted(&m.target)));
            line(&format!("  writable: {}", m.writable));
        }
    }

    line("containerd:");
    line("  system: false");
    line("  user: false");
    line("ssh:");
    line("  forwardAgent: false");
    line("  loadDotSSHPubKeys: false");
    line("propagateProxyEnv: false");
    line("user:");
    line(&format!("  name: {GUEST_USER}"));
    line("  comment: \"\"");

    // Rules are checked in order and the first match wins: the ports the
    // spec lists, then one rule per guest address that ignores every other
    // port. Lima's own fallback rule comes after these and never matches.
    line("portForwards:");
    for p in &spec.ports {
        line(&format!("- guestPort: {}", p.guest));
        line(&format!("  hostPort: {}", p.host));
        line(&format!("  hostIP: {}", quoted(&p.bind.to_string())));
        line("  proto: tcp");
    }
    for guest_ip in ["127.0.0.1", "0.0.0.0", "::"] {
        line(&format!("- guestIP: {}", quoted(guest_ip)));
        line("  proto: any");
        line("  guestPortRange: [1, 65535]");
        line("  ignore: true");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use adytum_model::{ImagePin, Mount, Port, Resources, Wall};
    use std::path::PathBuf;

    fn spec(sealed: bool) -> CellSpec {
        CellSpec {
            name: "adytum-test-tpl".parse().unwrap(),
            driver: DriverKind::Lima,
            image: ImagePin::new(
                "https://cloud-images.ubuntu.com/releases/noble/release-20260518/ubuntu-24.04-server-cloudimg-arm64.img".into(),
                "sha256:6a61b967ba4a27dd1966f835a67643073ed55c2860ce3dc1cb0517282e6b8bec".into(),
                Arch::Aarch64,
            )
            .unwrap(),
            wall: "wall_a".parse::<Wall>().unwrap(),
            resources: Resources::default(),
            ports: vec![],
            automation_port: None,
            mounts: vec![],
            sealed,
            network: None,
            clone_from: None,
            secrets: vec![],
        }
    }

    fn mount() -> Mount {
        Mount { source: PathBuf::from("/srv/share"), target: "/mnt/share".into(), writable: false }
    }

    #[test]
    fn a_sealed_template_has_empty_mounts_and_the_ignore_rules_last() {
        let text = render(&spec(true)).unwrap();
        assert!(text.contains("\nmounts: []\n"), "{text}");
        assert!(text.ends_with("- guestIP: \"::\"\n  proto: any\n  guestPortRange: [1, 65535]\n  ignore: true\n"), "{text}");
        // 127.0.0.1 has its own rule: Lima's 0.0.0.0 rule alone does not match it.
        let rules: Vec<_> = text.lines().filter(|l| l.starts_with("- guestIP")).collect();
        assert_eq!(rules, ["- guestIP: \"127.0.0.1\"", "- guestIP: \"0.0.0.0\"", "- guestIP: \"::\""]);
    }

    #[test]
    fn a_sealed_template_matches_its_golden_file() {
        let golden = include_str!("../tests/golden/template-sealed.yaml");
        assert_eq!(render(&spec(true)).unwrap(), golden);
    }

    #[test]
    fn a_sealed_spec_that_smuggles_a_mount_is_refused_and_writes_nothing() {
        let mut s = spec(true);
        s.mounts.push(mount());
        let Err(DriverError::Refused(r)) = render(&s) else { panic!("not refused") };
        assert_eq!(r.code, adytum_model::RefusalCode::SealedCellMount);
    }

    #[test]
    fn an_unsealed_cell_gets_only_its_own_mounts() {
        let mut s = spec(false);
        s.mounts.push(mount());
        let text = render(&s).unwrap();
        assert!(text.contains("mounts:\n- location: \"/srv/share\"\n  mountPoint: \"/mnt/share\"\n  writable: false\n"), "{text}");
        assert!(!text.contains("mounts: []"));
    }

    #[test]
    fn published_ports_come_before_the_ignore_rules_and_bind_to_localhost() {
        let mut s = spec(true);
        s.ports.push(Port { guest: 8080, host: 18080, bind: "127.0.0.1".parse().unwrap() });
        let text = render(&s).unwrap();
        let port_at = text.find("guestPort: 8080").unwrap();
        let first_ignore = text.find("ignore: true").unwrap();
        assert!(port_at < first_ignore);
        assert!(text.contains("  hostPort: 18080\n  hostIP: \"127.0.0.1\"\n  proto: tcp\n"));
    }

    #[test]
    fn a_port_on_another_address_is_refused() {
        let mut s = spec(true);
        s.ports.push(Port { guest: 80, host: 8080, bind: "0.0.0.0".parse().unwrap() });
        let Err(DriverError::Refused(r)) = render(&s) else { panic!("not refused") };
        assert_eq!(r.code, adytum_model::RefusalCode::PortNotLocalhost);
    }

    #[test]
    fn the_automation_port_is_never_forwarded() {
        let mut s = spec(true);
        s.automation_port = Some(9222);
        let text = render(&s).unwrap();
        assert!(!text.contains("9222"));
        s.ports.push(Port { guest: 9222, host: 9222, bind: "127.0.0.1".parse().unwrap() });
        let Err(DriverError::Refused(r)) = render(&s) else { panic!("not refused") };
        assert_eq!(r.code, adytum_model::RefusalCode::AutomationPortPublished);
    }

    #[test]
    fn the_host_user_is_not_copied_into_the_guest() {
        let text = render(&spec(true)).unwrap();
        assert!(text.contains("user:\n  name: adytum\n  comment: \"\"\n"));
        assert!(text.contains("propagateProxyEnv: false"));
        assert!(text.contains("forwardAgent: false"));
        assert!(text.contains("containerd:\n  system: false\n  user: false\n"));
    }

    #[test]
    fn what_lima_cannot_honour_is_refused_not_dropped() {
        let mut s = spec(true);
        s.network = Some("net-1".parse().unwrap());
        assert!(matches!(render(&s), Err(DriverError::Unsupported { .. })));
        let mut s = spec(true);
        s.clone_from = Some("adytum-other".parse().unwrap());
        assert!(matches!(render(&s), Err(DriverError::Unsupported { .. })));
        let mut s = spec(true);
        s.driver = DriverKind::Podman;
        assert!(matches!(render(&s), Err(DriverError::Unsupported { .. })));
    }
}
