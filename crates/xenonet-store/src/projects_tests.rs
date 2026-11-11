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
        common: Some(format!("{}/.git", path.to_lowercase())),
        kind: CheckoutKind::Repo,
        parent: None,
        name: path.rsplit('/').next().unwrap_or(path).into(),
    }
}

fn folder(path: &str) -> Found {
    Found { vcs: None, branch: None, head: None, repo_key: None, common: None, kind: CheckoutKind::Folder, ..repo(path, "") }
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

// A linked worktree of `main`, whose git folder is `common`.
fn worktree(path: &str, main: &str, key: &str, common: &str) -> Found {
    Found { kind: CheckoutKind::Worktree, parent: Some(main.into()), common: Some(common.into()), ..repo(path, key) }
}

#[test]
fn a_folder_gets_a_project_of_its_own_and_a_worktree_is_a_candidate_after_its_main() {
    let db = db();
    // A worktree whose git folder isn't its main checkout's: it only names
    // the remote, so it waits for the person.
    let wt = worktree("/w/app/.claude/worktrees/x", "/w/app", "github.com/a/app", "/elsewhere/app/.git");
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

#[test]
fn a_scopes_project_terms_name_confirmed_checkouts_only() {
    let db = db();
    db.reconcile(&[repo("/w/atlas", "x/atlas"), repo("/w/copy/atlas", "x/atlas"), repo("/w/other", "x/ledger"), folder("/w/notes")], 1).unwrap();
    let atlas = db.place("/w/atlas").unwrap().unwrap().checkout.project;
    assert_eq!(db.confirmed_projects(Some("Atlas")).unwrap(), vec![atlas], "by its name, any case");
    let ledger = db.place("/w/other").unwrap().unwrap().checkout.project;
    assert_eq!(db.confirmed_projects(Some("other")).unwrap(), vec![ledger], "or its checkout's folder");
    assert_eq!(db.confirmed_projects(None).unwrap().len(), 3, "every project with a confirmed checkout");
    // A project whose only checkout here is a candidate is named by nothing.
    let lone = Db::memory().unwrap();
    lone.use_ids(Ids::seeded("lone"));
    lone.reconcile(&[repo("/w/a", "x/a")], 1).unwrap();
    lone.reconcile(&[repo("/w/b", "x/a")], 2).unwrap();
    assert!(lone.place("/w/a").unwrap().is_none(), "the founding checkout went");
    assert!(!lone.place("/w/b").unwrap().unwrap().checkout.confirmed, "the copy is a candidate");
    assert_eq!(lone.confirmed_projects(Some("b")).unwrap(), Vec::new(), "a candidate checkout gives no project rights");
}

#[test]
fn a_project_reads_through_its_confirmed_checkouts_only() {
    let db = db();
    db.reconcile(&[repo("/w/atlas", "x/atlas"), repo("/w/copy/atlas", "x/atlas")], 1).unwrap();
    let (main, copy) = (db.place("/w/atlas").unwrap().unwrap().checkout, db.place("/w/copy/atlas").unwrap().unwrap().checkout);
    assert_eq!(db.project_of_checkout(&main.id).unwrap(), Some(main.project.clone()));
    assert_eq!(db.project_of_checkout(&copy.id).unwrap(), None, "a candidate gives rights in nothing");
    let ids = |v: Vec<Checkout>| v.into_iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(ids(db.confirmed_checkouts_of(&main.project).unwrap()), vec![main.id.clone()]);
    db.confirm(&copy.id).unwrap();
    assert_eq!(ids(db.confirmed_checkouts_of(&main.project).unwrap()), vec![main.id.clone(), copy.id.clone()], "by path");
    // Gone, it reads nothing, and still says which project it was in.
    db.reconcile(&[repo("/w/copy/atlas", "x/atlas")], 2).unwrap();
    assert_eq!(ids(db.confirmed_checkouts_of(&main.project).unwrap()), vec![copy.id]);
    assert_eq!(db.project_of_checkout(&main.id).unwrap(), Some(main.project));
    assert_eq!(db.confirmed_checkouts_of(&ProjectId::parse("prj_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap()).unwrap(), Vec::new(), "one the store doesn't hold");
}

#[test]
fn a_worktree_sharing_its_confirmed_mains_git_folder_is_in_with_it() {
    let db = db();
    let wt = worktree("/w/app/.claude/worktrees/x", "/w/app", "github.com/a/app", "/w/app/.git");
    // One with a git folder of its own, beside it, and a main checkout of
    // the same remote elsewhere, a candidate, with its own worktree.
    let other = worktree("/w/app/.claude/worktrees/y", "/w/app", "github.com/a/app", "/tmp/y/.git");
    let copy = repo("/w/copy/app", "github.com/a/app");
    let copy_wt = worktree("/w/copy/app/wt", "/w/copy/app", "github.com/a/app", "/w/copy/app/.git");
    let fixes = db.reconcile(&[wt.clone(), other.clone(), repo("/w/app", "github.com/a/app"), copy.clone(), copy_wt.clone()], 1).unwrap();
    let at = |p: &str| db.place(p).unwrap().unwrap().checkout;
    let main = at("/w/app");
    assert!(main.confirmed, "the main checkout founds the project");
    let x = at("/w/app/.claude/worktrees/x");
    assert_eq!((x.confirmed, x.project.clone(), x.common.as_deref()), (true, main.project.clone(), Some("/w/app/.git")), "the same refs and objects: in with its main");
    assert!(fixes.iter().any(|f| matches!(f, Fix::Found { checkout, candidate: false, .. } if *checkout == x.id)), "{fixes:?}");
    assert!(!at("/w/app/.claude/worktrees/y").confirmed, "another git folder: the person's to confirm");
    assert!(!at("/w/copy/app").confirmed && !at("/w/copy/app/wt").confirmed, "a candidate's worktree waits with it");
    // Confirming the copy brings in its worktree that shares its git
    // folder, and nothing else.
    db.confirm(&at("/w/copy/app").id).unwrap();
    assert!(at("/w/copy/app/wt").confirmed);
    assert!(!at("/w/app/.claude/worktrees/y").confirmed);
    // Found again, it stays as it is.
    db.reconcile(&[wt, other, repo("/w/app", "github.com/a/app"), copy, copy_wt], 2).unwrap();
    assert!(at("/w/app/.claude/worktrees/x").confirmed && !at("/w/app/.claude/worktrees/y").confirmed);
}

#[test]
fn a_candidate_worktree_is_in_once_its_main_is() {
    let db = db();
    db.reconcile(&[repo("/w/app", "github.com/a/app")], 1).unwrap();
    // The same repo's second clone, a candidate, and a worktree of it
    // found later, by a session's folder.
    db.reconcile(&[repo("/w/app", "github.com/a/app"), repo("/w/clone", "github.com/a/app")], 2).unwrap();
    let (wt, _) = db.ensure(&worktree("/w/clone/wt", "/w/clone", "github.com/a/app", "/w/clone/.git"), 3).unwrap();
    assert!(!wt.confirmed, "its main checkout is a candidate");
    let clone = db.place("/w/clone").unwrap().unwrap().checkout;
    // The person confirms the clone outside a reconcile; the worktree's
    // next sighting brings it in.
    db.write(|c| Ok(c.execute("UPDATE checkouts SET confirmed = 1 WHERE id = ?1", params![clone.id.as_str()])?)).unwrap();
    let (wt, _) = db.ensure(&worktree("/w/clone/wt", "/w/clone", "github.com/a/app", "/w/clone/.git"), 4).unwrap();
    assert!(wt.confirmed);
}
