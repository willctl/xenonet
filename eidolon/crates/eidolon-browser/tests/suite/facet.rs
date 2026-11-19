//! The browser facet: what it leaves out, and a profile sealed in one cell
//! and opened in another.

use crate::support::RecordingCell;
use eidolon_browser::{BrowserFacet, PROFILE_LEAVE_OUT};
use eidolon_facet::{open_facets, seal_facets, Facet, FacetError};
use eidolon_model::{EidolonId, FacetId, FacetKind, FacetSpec};
use eidolon_pack::glob::matches;
use eidolon_vault::{EidolonKey, Keys, Vault};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use xenonet_id::codec::{hex, random};
use xenonet_id::sign::{KeyHome, SeedSigner};

const PROFILE: &str = "/home/eidolon/profile";

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        let dir = std::env::temp_dir().join(format!("eidolon-browser-test-{}", hex(&random::<8>().unwrap())));
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn spec(leave_out: &[&str]) -> FacetSpec {
    FacetSpec {
        id: FacetId::from_bytes([6; 16]),
        kind: FacetKind::Browser,
        name: "browser".into(),
        path: PROFILE.into(),
        leave_out: leave_out.iter().map(|s| (*s).to_owned()).collect(),
    }
}

fn facet(leave_out: &[&str]) -> BrowserFacet {
    BrowserFacet::new(spec(leave_out)).unwrap().with_retry_gap(Duration::ZERO)
}

#[test]
fn the_leave_out_list_matches_the_profiles_caches_and_nothing_it_must_keep() {
    let patterns = facet(&[]).leave_out();
    let left_out = |path: &str| patterns.iter().any(|p| matches(p, path));
    for gone in [
        "cache2",
        "cache2/entries/ABCDEF",
        "startupCache/startupCache.8.little",
        "thumbnails/1.png",
        "crashes/events/e1",
        "minidumps/a.dmp",
        "datareporting/state.json",
        "saved-telemetry-pings/ping",
        "lock",
        ".parentlock",
        "extensions/.parentlock",
        "storage.sqlite.lock",
    ] {
        assert!(left_out(gone), "{gone} is rebuilt by the browser and must be left out");
    }
    for kept in [
        "cookies.sqlite",
        "cookies.sqlite-wal",
        "key4.db",
        "logins.json",
        "prefs.js",
        "user.js",
        "places.sqlite",
        "sessionstore-backups/recovery.baklz4",
        "storage/default/https+++x/ls/data.sqlite",
        "extensions/a.xpi",
    ] {
        assert!(!left_out(kept), "{kept} is the identity and must be kept");
    }
    assert_eq!(patterns.len(), PROFILE_LEAVE_OUT.len());
}

#[test]
fn the_specs_own_leave_out_is_added_once() {
    let patterns = facet(&["cache2/**", "tmp/**"]).leave_out();
    assert_eq!(patterns.iter().filter(|p| *p == "cache2/**").count(), 1);
    assert!(patterns.contains(&"tmp/**".to_owned()));
}

#[test]
fn a_facet_refuses_a_folder_spec_and_a_leave_out_that_escapes() {
    let mut folder = spec(&[]);
    folder.kind = FacetKind::Folder;
    assert!(matches!(BrowserFacet::new(folder), Err(FacetError::WrongKind { .. })));
    assert!(matches!(BrowserFacet::new(spec(&["../elsewhere/**"])), Err(FacetError::Refused(_))));
}

#[tokio::test]
async fn quiet_stops_the_units_and_waits_for_the_answer_not_for_a_timer() {
    let s = Scratch::new();
    // The processes are still there for two asks, then gone.
    let cell = RecordingCell::new(s.path("cell"), &[0, 0, 1]);
    facet(&[]).quiet(&cell).await.unwrap();
    let commands = cell.commands();
    assert!(commands[0].starts_with("sudo systemctl stop eidolon-browser.service eidolon-view.service"), "{commands:?}");
    assert_eq!(commands.iter().filter(|c| *c == "pgrep -u eidolon").count(), 3);
}

#[tokio::test]
async fn quiet_gives_up_after_a_bounded_number_of_asks() {
    let s = Scratch::new();
    let cell = RecordingCell::new(s.path("cell"), &[0; 200]);
    let err = facet(&[]).quiet(&cell).await.unwrap_err();
    assert!(err.to_string().contains("did not stop"), "{err}");
    let asks = cell.commands().iter().filter(|c| c.starts_with("pgrep")).count();
    assert!((1..200).contains(&asks), "asked {asks} times");
}

#[tokio::test]
async fn quiet_fails_when_the_units_cannot_be_stopped() {
    let s = Scratch::new();
    let cell = RecordingCell::new(s.path("cell"), &[]).failing("systemctl");
    assert!(facet(&[]).quiet(&cell).await.is_err());
    assert!(!cell.commands().iter().any(|c| c.starts_with("pgrep")), "it did not go on to ask");
}

#[tokio::test]
async fn resume_starts_the_automation_browser() {
    let s = Scratch::new();
    let cell = RecordingCell::new(s.path("cell"), &[]);
    facet(&[]).resume(&cell).await.unwrap();
    assert_eq!(cell.commands(), ["sudo systemctl start eidolon-browser.service"]);
}

fn fill_profile(root: &std::path::Path) {
    for dir in ["cache2/entries", "startupCache", "sessionstore-backups", "crashes", "storage/default/site"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(root.join("cookies.sqlite"), b"cookie jar").unwrap();
    fs::write(root.join("key4.db"), b"keys").unwrap();
    fs::write(root.join("user.js"), b"user_pref(\"a\", 1);").unwrap();
    fs::write(root.join("cache2/entries/AB"), b"cache").unwrap();
    fs::write(root.join("startupCache/x"), b"cache").unwrap();
    fs::write(root.join("crashes/c"), b"crash").unwrap();
    fs::write(root.join("sessionstore-backups/recovery.baklz4"), b"tabs").unwrap();
    fs::write(root.join("storage/default/site/data"), b"local storage").unwrap();
    fs::write(root.join(".parentlock"), b"").unwrap();
    std::os::unix::fs::symlink("10.0.0.1:+123", root.join("lock")).unwrap();
}

#[tokio::test]
async fn a_profile_is_sealed_without_its_caches_and_opened_with_its_cookies() {
    let s = Scratch::new();
    let (a, b) = (RecordingCell::new(s.path("cell-a"), &[1]), RecordingCell::new(s.path("cell-b"), &[]));
    fill_profile(&s.path("cell-a/home/eidolon/profile"));
    let vault = Vault::open(&s.path("vault")).unwrap();
    let signer = SeedSigner::from_seed(&[8; 32], KeyHome::File).unwrap();
    let keys = Keys::new(&EidolonKey::from_bytes([3; 32])).unwrap();
    let facets: Vec<Box<dyn Facet>> = vec![Box::new(facet(&[]))];
    let head = seal_facets(&facets, &a, &keys, &vault, &signer, EidolonId::from_bytes([1; 16]), 2, &s.path("work")).await.unwrap();

    // The order of what ran in the cell: stop, look, open, close, start.
    let ran = a.commands();
    let position = |needle: &str| ran.iter().position(|c| c.contains(needle)).unwrap_or(usize::MAX);
    assert!(["systemctl stop", "pgrep", "a+rX", "u+rwX,go-rwx", "systemctl start"].iter().all(|n| position(n) != usize::MAX), "{ran:?}");
    assert!(position("systemctl stop") < position("pgrep"));
    assert!(position("pgrep") < position("a+rX"));
    assert!(position("a+rX") < position("u+rwX,go-rwx"));
    assert!(position("u+rwX,go-rwx") < position("systemctl start eidolon-browser.service"));

    open_facets(&head, &facets, &b, &keys, &vault, &s.path("work")).await.unwrap();
    let there = s.path("cell-b/home/eidolon/profile");
    assert_eq!(fs::read(there.join("cookies.sqlite")).unwrap(), b"cookie jar");
    assert_eq!(fs::read(there.join("key4.db")).unwrap(), b"keys");
    assert_eq!(fs::read(there.join("storage/default/site/data")).unwrap(), b"local storage");
    assert_eq!(fs::read(there.join("sessionstore-backups/recovery.baklz4")).unwrap(), b"tabs");
    for gone in ["cache2", "startupCache", "crashes", ".parentlock", "lock"] {
        assert!(fs::symlink_metadata(there.join(gone)).is_err(), "{gone} should not have been carried");
    }
    // The copy arrived owned by the cell's user, so the facet hands it over.
    let placed = b.commands();
    assert_eq!(placed, PLACE_ARGV);
}

/// What placing a non-empty profile runs in the cell, in order.
const PLACE_ARGV: [&str; 7] = [
    "sudo rm -rf -- /home/eidolon/profile.incoming",
    "id -un",
    "sudo install -d -o builder -m 0700 /home/eidolon/profile.incoming",
    "sudo rm -rf -- /home/eidolon/profile",
    "sudo mv -- /home/eidolon/profile.incoming /home/eidolon/profile",
    "sudo chown -R eidolon:eidolon /home/eidolon/profile",
    "sudo chmod -R u+rwX,go-rwx /home/eidolon/profile",
];

#[tokio::test]
async fn placing_a_profile_replaces_the_one_in_the_cell_whole() {
    let s = Scratch::new();
    let cell = RecordingCell::new(s.path("cell"), &[]);
    let staged = s.path("staged");
    fill_profile(&staged);
    // The cell already has a profile with a browser's journal files in it.
    let old = s.path("cell/home/eidolon/profile");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("cookies.sqlite-wal"), b"journal").unwrap();
    facet(&[]).place(&cell, &staged).await.unwrap();
    let mut expected = PLACE_ARGV.to_vec();
    expected.pop();
    assert_eq!(cell.commands(), PLACE_ARGV);
    assert!(!old.join("cookies.sqlite-wal").exists(), "the browser's journal is gone");
    assert_eq!(fs::read(old.join("cookies.sqlite")).unwrap(), b"cookie jar");
    assert!(!s.path("cell/home/eidolon/profile.incoming").exists());
}

#[tokio::test]
async fn an_empty_staged_profile_places_nothing() {
    let s = Scratch::new();
    let cell = RecordingCell::new(s.path("cell"), &[]);
    fs::create_dir_all(s.path("staged")).unwrap();
    facet(&[]).place(&cell, &s.path("staged")).await.unwrap();
    assert!(cell.commands().is_empty(), "{:?}", cell.commands());
    assert!(!s.path("cell/home/eidolon").exists());
}

#[tokio::test]
async fn a_leftover_incoming_folder_is_removed_before_anything_is_made() {
    let s = Scratch::new();
    let cell = RecordingCell::new(s.path("cell"), &[]);
    fill_profile(&s.path("staged"));
    let stale = s.path("cell/home/eidolon/profile.incoming");
    fs::create_dir_all(&stale).unwrap();
    fs::write(stale.join("half-copied"), b"x").unwrap();
    facet(&[]).place(&cell, &s.path("staged")).await.unwrap();
    assert!(!s.path("cell/home/eidolon/profile/half-copied").exists());
    let commands = cell.commands();
    assert_eq!(commands[0], "sudo rm -rf -- /home/eidolon/profile.incoming");
    assert!(commands[2].starts_with("sudo install -d"));
}

#[tokio::test]
async fn a_failed_swap_stops_before_the_old_profile_is_touched() {
    let s = Scratch::new();
    // Making the incoming folder fails: the old profile must not be removed.
    let cell = RecordingCell::new(s.path("cell"), &[]).failing("install");
    fill_profile(&s.path("staged"));
    assert!(facet(&[]).place(&cell, &s.path("staged")).await.is_err());
    assert!(!cell.commands().iter().any(|c| c == "sudo rm -rf -- /home/eidolon/profile"));
}

#[tokio::test]
async fn a_failed_copy_still_closes_the_profile_again() {
    let s = Scratch::new();
    // No profile folder in the cell, so the copy fails.
    let cell = RecordingCell::new(s.path("cell"), &[]);
    let err = facet(&[]).take(&cell, &s.path("staging")).await;
    assert!(err.is_err());
    assert!(cell.commands().last().unwrap().contains("go-rwx"));
}
