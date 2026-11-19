//! The doctor against described machines, and once against the real one.

use std::path::{Path, PathBuf};

use adytum_doctor::{checks, has_failure, render_human, render_json, run, Check, LedgerStatus, Level, Probe, KNOWN_LIMA_VERSIONS};
use async_trait::async_trait;

/// A machine that is fine, with knobs to break it.
#[derive(Clone)]
struct Machine {
    limactl: Option<String>,
    home: Option<PathBuf>,
    lima_home_exists: bool,
    cached: bool,
    free: Option<u64>,
    ledger: LedgerStatus,
    podman: Option<String>,
    docker: Option<String>,
}

const GIB: u64 = 1024 * 1024 * 1024;

fn fine() -> Machine {
    Machine {
        limactl: Some(KNOWN_LIMA_VERSIONS[0].to_owned()),
        home: Some(PathBuf::from("/Users/desk/.adytum")),
        lima_home_exists: true,
        cached: true,
        free: Some(200 * GIB),
        ledger: LedgerStatus::Open { unsettled: 0 },
        podman: None,
        docker: None,
    }
}

#[async_trait]
impl Probe for Machine {
    async fn limactl_version(&self) -> Option<String> {
        self.limactl.clone()
    }
    fn adytum_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
    fn exists(&self, _path: &Path) -> bool {
        self.lima_home_exists
    }
    async fn free_bytes(&self, _path: &Path) -> Option<u64> {
        self.free
    }
    fn lima_cache_has(&self, _location: &str) -> bool {
        self.cached
    }
    fn ledger(&self, _home: &Path) -> LedgerStatus {
        self.ledger.clone()
    }
    async fn tool_version(&self, tool: &str) -> Option<String> {
        if tool == "podman" {
            self.podman.clone()
        } else {
            self.docker.clone()
        }
    }
}

fn get<'a>(all: &'a [Check], id: &str) -> &'a Check {
    all.iter().find(|c| c.id == id).expect("every scenario has this check")
}

#[tokio::test]
async fn a_fine_machine_has_nothing_to_do_and_still_says_what_a_container_is() {
    let all = run(&fine()).await;
    assert!(all.iter().all(|c| c.ok), "{all:?}");
    assert!(!has_failure(&all));
    let wall = get(&all, "container-wall");
    assert_eq!(wall.level, Level::Info);
    assert!(wall.message.contains("weaker wall than a VM") && wall.message.contains("shares this machine's kernel"));
}

#[tokio::test]
async fn limactl_missing_fails_and_an_unknown_version_warns_with_the_known_ones() {
    let all = run(&Machine { limactl: None, ..fine() }).await;
    assert_eq!(get(&all, "limactl").level, Level::Fail);
    assert!(has_failure(&all));

    let all = run(&Machine { limactl: Some("9.9.9".to_owned()), ..fine() }).await;
    let c = get(&all, "limactl");
    assert_eq!(c.level, Level::Warn);
    assert!(c.message.contains("9.9.9") && c.message.contains(KNOWN_LIMA_VERSIONS[0]), "{}", c.message);
    assert!(!has_failure(&all));
}

#[tokio::test]
async fn a_long_adytum_folder_warns_and_then_fails_before_lima_hits_the_socket_limit() {
    // 104 bytes less "/", a 54-character name, "/default_ep.sock" and NUL leaves 32 for the Lima folder.
    let at = |n: usize| Machine { home: Some(PathBuf::from(format!("/{}", "a".repeat(n - "/lima".len() - 1)))), ..fine() };
    assert!(get(&run(&at(32)).await, "socket-path").ok);
    assert_eq!(get(&run(&at(33)).await, "socket-path").level, Level::Warn);
    assert_eq!(get(&run(&at(80)).await, "socket-path").level, Level::Fail);
    let fix = get(&run(&at(80)).await, "socket-path").fix.clone().unwrap();
    assert!(fix.contains("ADYTUM_HOME"));
}

#[tokio::test]
async fn an_image_that_is_not_cached_is_a_note_with_a_fix_not_a_failure() {
    let all = run(&Machine { cached: false, ..fine() }).await;
    let c = get(&all, "image-cache");
    assert_eq!((c.ok, c.level), (false, Level::Info));
    assert!(c.message.contains("first create downloads it"));
    assert!(!has_failure(&all));
    let all = run(&Machine { lima_home_exists: false, ..fine() }).await;
    assert_eq!(get(&all, "lima-home").level, Level::Info);
}

#[tokio::test]
async fn under_20_gib_free_fails_with_the_number() {
    let all = run(&Machine { free: Some(5 * GIB + GIB / 2), ..fine() }).await;
    let c = get(&all, "disk");
    assert_eq!(c.level, Level::Fail);
    assert!(c.message.contains("5.5 GiB"), "{}", c.message);
    assert!(!run(&Machine { free: Some(20 * GIB), ..fine() }).await.iter().any(|c| c.id == "disk" && !c.ok));
    assert_eq!(get(&run(&Machine { free: None, ..fine() }).await, "disk").level, Level::Warn);
}

#[tokio::test]
async fn a_ledger_to_set_up_again_fails_and_unsettled_operations_warn() {
    let all = run(&Machine { ledger: LedgerStatus::SetUpAgain("it was made by another version".to_owned()), ..fine() }).await;
    let c = get(&all, "ledger");
    assert_eq!(c.level, Level::Fail);
    assert!(c.message.contains("another version") && c.fix.as_deref().unwrap().contains("Set Adytum up again"));
    assert!(!all.iter().any(|c| c.id == "ledger-settled"), "no count for a ledger that does not open");

    let all = run(&Machine { ledger: LedgerStatus::Open { unsettled: 2 }, ..fine() }).await;
    let c = get(&all, "ledger-settled");
    assert_eq!(c.level, Level::Warn);
    assert_eq!(c.fix.as_deref(), Some("Run any adytum command to finish them."));
    assert!(get(&all, "ledger").ok);

    assert!(get(&run(&Machine { ledger: LedgerStatus::Missing, ..fine() }).await, "ledger").ok);
}

#[tokio::test]
async fn host_containers_are_never_a_problem() {
    for (podman, docker) in [(None, None), (Some("5.3.1"), None), (Some("5.3.1"), Some("27.3.1"))] {
        let m = Machine { podman: podman.map(str::to_owned), docker: docker.map(str::to_owned), ..fine() };
        let all = run(&m).await;
        let c = get(&all, "host-containers");
        assert_eq!(c.level, Level::Info);
        assert!(c.ok && c.message.contains("Not needed"), "{}", c.message);
        assert_eq!(c.message.contains("podman 5.3.1"), podman.is_some());
        assert_eq!(c.message.contains("docker 27.3.1"), docker.is_some());
    }
}

#[tokio::test]
async fn no_home_fails_and_every_check_that_is_not_ok_has_a_fix() {
    let machines = [
        Machine { home: None, ..fine() },
        Machine { limactl: None, cached: false, free: Some(GIB), ledger: LedgerStatus::Open { unsettled: 1 }, lima_home_exists: false, ..fine() },
        Machine { ledger: LedgerStatus::Unreadable("disk I/O error".to_owned()), free: None, limactl: Some("0.1".to_owned()), ..fine() },
        Machine { home: Some(PathBuf::from(format!("/{}", "x".repeat(70)))), ..fine() },
    ];
    let mut not_ok = 0;
    for m in machines {
        for c in run(&m).await {
            if !c.ok {
                not_ok += 1;
                assert!(c.fix.as_deref().is_some_and(|f| f.len() > 10), "{} has no fix", c.id);
            }
        }
    }
    assert!(not_ok >= 8, "{not_ok}");
    assert!(has_failure(&run(&Machine { home: None, ..fine() }).await));
}

#[tokio::test]
async fn the_answer_prints_for_people_and_for_scripts() {
    let all = run(&Machine { limactl: Some("9.9.9".to_owned()), free: Some(GIB), cached: false, ..fine() }).await;
    let text = render_human(&all);
    assert!(text.lines().any(|l| l.starts_with("● ok")), "{text}");
    assert!(text.lines().any(|l| l.starts_with("▲ warn") && l.contains("limactl")), "{text}");
    assert!(text.lines().any(|l| l.starts_with("✕ fail") && l.contains("disk")), "{text}");
    assert!(text.contains("       fix: "));
    // A check that is ok has no fix line under it.
    let ok_only = render_human(&run(&fine()).await);
    assert!(!ok_only.contains("fix:"));

    let json: serde_json::Value = serde_json::from_str(&render_json(&all)).unwrap();
    assert_eq!(json["ok"], false);
    let first = &json["checks"][0];
    assert_eq!(first["id"], "limactl");
    assert_eq!(first["level"], "warn");
    assert_eq!(first["ok"], false);
    assert!(first["fix"].is_string());
    assert!(json["checks"][json["checks"].as_array().unwrap().len() - 1]["fix"].is_null());
    let fine_json: serde_json::Value = serde_json::from_str(&render_json(&run(&fine()).await)).unwrap();
    assert_eq!(fine_json["ok"], true);
}

#[test]
fn every_known_lima_version_has_a_golden_file_in_the_lima_crate() {
    for v in KNOWN_LIMA_VERSIONS {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../adytum-lima/tests/golden").join(format!("lima-{v}"));
        assert!(dir.is_dir(), "{} is listed as known but has no golden folder", dir.display());
    }
}

#[tokio::test]
async fn the_real_machine_answers_without_error() {
    // Only that it returns and says the always-present things; what the machine
    // holds is its own business.
    let all = checks().await;
    assert_ne!(all.len(), 0);
    assert!(all.iter().any(|c| c.id == "container-wall"));
    assert!(all.iter().all(|c| c.ok || c.fix.is_some()));
    assert_ne!(render_human(&all).len(), 0);
}
