use super::*;

fn db() -> Db {
    let db = Db::memory().unwrap();
    db.use_ids(Ids::seeded("projects"));
    db
}

fn repo(path: &str, key: &str) -> Found {
    Found {
        path: path.into(),
        path_key: path.to_lowercase(),
        vcs: Some(".git".into()),
        branch: Some("main".into()),
        head: Some("abc".into()),
        repo_key: Some(key.into()),
        kind: CheckoutKind::Repo,
        parent: None,
        name: path.rsplit('/').next().unwrap_or(path).into(),
    }
}

fn folder(path: &str) -> Found {
    Found { vcs: None, branch: None, head: None, repo_key: None, kind: CheckoutKind::Folder, ..repo(path, "") }
}

fn owned(db: &Db, person: &str, salt: u8) {
    let person = person.to_string();
    db.set_owner(Arc::new(move || Owner { person: Some(person.clone()), salt: Some([salt; 32]) }));
}

#[test]
fn the_first_checkout_founds_its_project_and_the_next_waits_for_the_person() {
    let db = db();
    owned(&db, "prs_a", 7);
    let fixes = db.reconcile(&[repo("/w/atlas", "github.com/a/atlas"), repo("/w/atlas-copy", "github.com/a/atlas")], 1).unwrap();
    let all = db.checkouts().unwrap();
    let (first, copy) = (&all[0], &all[1]);
    assert_eq!(first.project, ProjectId::of_repo(&[7; 32], "github.com/a/atlas"), "a repo's id is its owner's HMAC of the remote");
    assert_eq!(first.project, copy.project, "one repo, one project");
    assert!(first.confirmed && !copy.confirmed, "the first founds it; the second is a candidate");
    assert!(matches!(&fixes[1], Fix::Found { candidate: true, .. }), "{fixes:?}");
    assert!(fixes[1].words().contains("once you confirm it"));
    // Seen again, nothing changes; confirmed by the person, it's in.
    assert_eq!(db.reconcile(&[repo("/w/atlas", "github.com/a/atlas"), repo("/w/atlas-copy", "github.com/a/atlas")], 2).unwrap(), Vec::new());
    assert!(db.confirm(&copy.id).unwrap().confirmed);
    assert!(db.checkout(&copy.id).unwrap().unwrap().confirmed);
}

#[test]
fn a_folder_gets_a_project_of_its_own_and_a_worktree_is_a_candidate_after_its_main() {
    let db = db();
    let mut wt = repo("/w/app/.claude/worktrees/x", "github.com/a/app");
    wt.kind = CheckoutKind::Worktree;
    wt.parent = Some("/w/app".into());
    // Listed before its main checkout, the worktree still comes second.
    db.reconcile(&[wt, repo("/w/app", "github.com/a/app"), folder("/w/notes")], 1).unwrap();
    let app = db.place("/w/app/src/main.rs").unwrap().unwrap();
    assert_eq!((app.checkout.path_key.as_str(), app.rel.as_str(), app.checkout.confirmed), ("/w/app", "src/main.rs", true));
    let w = db.place("/w/app/.claude/worktrees/x/src/main.rs").unwrap().unwrap();
    assert_eq!((w.checkout.kind, w.rel.as_str(), w.checkout.confirmed), (CheckoutKind::Worktree, "src/main.rs", false), "the longest path key, a candidate");
    assert_eq!(w.checkout.parent.as_ref(), Some(&app.checkout.id));
    let notes = db.place("/w/notes").unwrap().unwrap();
    assert_eq!((notes.rel.as_str(), notes.checkout.kind), ("", CheckoutKind::Folder));
    assert_ne!(notes.checkout.project, app.checkout.project);
    assert!(notes.checkout.confirmed, "a folder founds its own project");
    assert!(db.place("/elsewhere").unwrap().is_none());
    assert!(db.place("/w/app-other").unwrap().is_none(), "a path key's parent, not its prefix");
}

#[test]
fn what_isnt_found_is_gone_and_comes_back_as_it_was() {
    let db = db();
    db.reconcile(&[repo("/w/a", "x/a"), repo("/w/b", "x/b")], 1).unwrap();
    let fixes = db.reconcile(&[repo("/w/a", "x/a")], 2).unwrap();
    assert!(matches!(&fixes[..], [Fix::Gone { path, .. }] if path == "/w/b"), "{fixes:?}");
    assert!(db.place("/w/b/x").unwrap().is_none(), "a gone checkout places nothing");
    let mut moved = repo("/w/b", "x/b");
    moved.branch = Some("dev".into());
    let fixes = db.reconcile(&[repo("/w/a", "x/a"), moved.clone()], 3).unwrap();
    assert!(matches!(&fixes[..], [Fix::Back { .. }]), "{fixes:?}");
    assert!(db.place("/w/b").unwrap().unwrap().checkout.confirmed, "it keeps what it was");
    moved.branch = Some("main".into());
    assert!(matches!(&db.reconcile(&[repo("/w/a", "x/a"), moved], 4).unwrap()[..], [Fix::Moved { .. }]));
}

#[test]
fn a_folder_that_gets_a_remote_joins_the_repos_project() {
    let db = db();
    db.reconcile(&[repo("/w/a", "x/a"), folder("/w/a2")], 1).unwrap();
    let before = db.place("/w/a2").unwrap().unwrap().checkout;
    let fixes = db.reconcile(&[repo("/w/a", "x/a"), repo("/w/a2", "x/a")], 2).unwrap();
    assert!(matches!(&fixes[..], [Fix::Rejoined { candidate: true, .. }]), "{fixes:?}");
    let after = db.place("/w/a2").unwrap().unwrap().checkout;
    assert_eq!(after.id, before.id, "the same checkout");
    assert!(!after.confirmed, "it joins an existing project only by the person's word");
    assert_eq!(db.resolve(&before.project).unwrap(), Some(after.project), "the folder's own project points at the repo's");
}

#[test]
fn an_ownerless_project_is_rekeyed_once_its_owner_is_known() {
    let db = db();
    db.reconcile(&[repo("/w/a", "x/a")], 1).unwrap();
    let before = db.place("/w/a").unwrap().unwrap().checkout.project;
    owned(&db, "prs_b", 3);
    let fixes = db.reconcile(&[repo("/w/a", "x/a")], 2).unwrap();
    let to = ProjectId::of_repo(&[3; 32], "x/a");
    assert!(fixes.contains(&Fix::Rekeyed { from: before.clone(), to: to.clone() }), "{fixes:?}");
    let ch = db.place("/w/a").unwrap().unwrap().checkout;
    assert_eq!(ch.project, to);
    assert!(ch.confirmed, "it was in its project, and it still is");
    assert_eq!(db.resolve(&before).unwrap(), Some(to.clone()), "the old id resolves to the new");
    assert_eq!(db.resolve(&to).unwrap(), Some(to));
    assert_eq!(db.resolve(&ProjectId::from_bytes(&[9; 20])).unwrap(), None);
}

#[test]
fn a_merge_moves_checkouts_and_resolve_follows_it() {
    let db = db();
    db.reconcile(&[folder("/w/a"), folder("/w/b")], 1).unwrap();
    let a = db.place("/w/a").unwrap().unwrap().checkout;
    let b = db.place("/w/b").unwrap().unwrap().checkout;
    db.merge(&a.project, &b.project).unwrap();
    assert_eq!(db.resolve(&a.project).unwrap(), Some(b.project.clone()));
    assert_eq!(db.place("/w/a").unwrap().unwrap().checkout.project, b.project);
    db.merge(&b.project, &a.project).unwrap();
    assert_eq!(db.resolve(&a.project).unwrap(), Some(b.project.clone()), "merging into what stands for it changes nothing");
    assert!(db.merge(&a.project, &ProjectId::from_bytes(&[1; 20])).is_err());
}

#[test]
fn ensure_makes_one_checkout_and_leaves_the_rest() {
    let db = db();
    db.reconcile(&[repo("/w/a", "x/a")], 1).unwrap();
    let (ch, fixes) = db.ensure(&folder("/tmp/scratch"), 2).unwrap();
    assert_eq!(ch.kind, CheckoutKind::Folder);
    assert_eq!(fixes.len(), 1);
    assert!(db.place("/w/a").unwrap().is_some(), "ensure marks nothing gone");
    assert!(db.confirm(&CheckoutId::from_bytes(&[2; 20])).is_err());
}

#[test]
fn seeded_ids_are_the_same_each_run() {
    let ids = |seed: &str| {
        let db = Db::memory().unwrap();
        db.use_ids(Ids::seeded(seed));
        db.reconcile(&[folder("/w/a"), repo("/w/b", "x/b")], 1).unwrap();
        db.checkouts().unwrap().into_iter().map(|c| (c.id, c.project)).collect::<Vec<_>>()
    };
    assert_eq!(ids("s"), ids("s"));
    assert_ne!(ids("s"), ids("t"));
}
