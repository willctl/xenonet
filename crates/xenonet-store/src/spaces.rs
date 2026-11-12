//! What the map knows to tie a space of the shared context to a project
//! by (xenolith/FORMAT.md, "The space marker"): the context keys
//! Xenolith's grants name, and a marker checked against them and the
//! projects' owners.
//!
//! A space folder's marker is a file anyone can write, so it ties nothing
//! by itself. The map ties a project space by its marker's `prj_` only
//! when the marker's signature holds by a context key one of this
//! machine's actors' grants names (`key(id)`: Xenolith's, recorded when a
//! forward of its grant is decided), and the space counts as the
//! project's: owned by the person who owns the project here, and signed by
//! them, not only named by its founder. Any other marker shows the space
//! by its folder's name.

use crate::{Db, R};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use xenonet_log::space::marker::{self, Marker};
use xenonet_model::ids::{EntityRef, ProjectId};

pub(crate) const TABLES: &[&str] = &[
    // The context keys this machine's actors' grants name (`key(id)`), by
    // the key's id: the actor, the grant's revocation ids (a JSON list of
    // hex) and the end of its time. A key counts while its grant is in its
    // time and none of its ids is revoked.
    "CREATE TABLE context_keys (
        key TEXT PRIMARY KEY,
        actor TEXT NOT NULL,
        ids TEXT NOT NULL,
        until INTEGER NOT NULL,
        at INTEGER NOT NULL
    ) STRICT",
    // The forward grants the person approved at the window, one per
    // actor: the ALPN it receives, the context key it speaks with, the
    // networks it reaches and its rights (JSON lists), when they approved
    // it, and the grant minted under it now: its first revocation id, its
    // revocation ids (a JSON list) and its end. The actor collects a new
    // one over the forward socket while it's within that end.
    "CREATE TABLE forward_grants (
        actor TEXT PRIMARY KEY,
        alpn TEXT NOT NULL,
        key TEXT NOT NULL,
        networks TEXT NOT NULL,
        rights TEXT NOT NULL,
        approved INTEGER NOT NULL,
        token TEXT,
        ids TEXT,
        until INTEGER
    ) STRICT",
];

/// A forward grant the person approved (`forward_grants`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardGrant {
    pub actor: String,
    pub alpn: String,
    pub key: String,
    pub networks: Vec<String>,
    pub rights: Vec<String>,
    pub approved: i64,
    /// The grant minted under it now: its first revocation id, all its
    /// ids and its end; None before the actor first collects one.
    pub token: Option<(String, Vec<String>, i64)>,
}

impl ForwardGrant {
    /// As the window lists it: never a token's text, which XenoNet doesn't
    /// keep.
    pub fn to_json(&self) -> Value {
        json!({
            "actor": self.actor,
            "alpn": self.alpn,
            "key": self.key,
            "networks": self.networks,
            "rights": self.rights,
            "approved": self.approved,
            "until": self.token.as_ref().map(|t| t.2),
        })
    }
}

fn list_of(text: Option<String>) -> Vec<String> {
    text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// A marker as the map takes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tie {
    /// Not a marker to go by (unsigned, edited, signed by a key no grant
    /// here names): the space shows by its folder's name.
    ByName,
    /// A marker to go by, of a space made under a name and not yet bound:
    /// by its name too.
    Unbound(Marker),
    /// The project's space: tied to the project by its id, resolved
    /// through merges.
    Project(Marker, ProjectId),
    /// About an entity it doesn't count as the space of here (its owner
    /// isn't the entity's, or never signed in it): "about E, owned by X",
    /// tied to nothing.
    Other(Marker),
}

impl Tie {
    /// `{ tie: name | id | none, … }`, as the map takes it.
    pub fn to_json(&self) -> Value {
        match self {
            Tie::ByName => json!({ "tie": "name" }),
            Tie::Unbound(m) => json!({ "tie": "name", "space": m.space, "about": m.about, "owner": m.owner }),
            Tie::Project(m, p) => json!({ "tie": "id", "space": m.space, "about": m.about, "owner": m.owner, "project": p.as_str() }),
            Tie::Other(m) => json!({ "tie": "none", "space": m.space, "about": m.about, "owner": m.owner }),
        }
    }
}

impl Db {
    /// Records `key`, the context key an actor's grant names, with the
    /// grant's revocation ids and end; a later grant naming it replaces it.
    pub fn set_context_key(&self, key: &str, actor: &str, ids: &[String], until: i64, at: i64) -> R<()> {
        let ids = Value::from(ids.to_vec()).to_string();
        self.write(|c| {
            c.execute(
                "INSERT INTO context_keys (key, actor, ids, until, at) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (key) DO UPDATE SET actor = ?2, ids = ?3, until = ?4, at = ?5 WHERE ids <> ?3 OR until <> ?4 OR actor <> ?2",
                params![key, actor, ids, until, at],
            )?;
            Ok(())
        })
    }

    /// Whether `key` is a context key a grant of this machine's names, in
    /// its time at `now` and not revoked.
    pub fn context_key(&self, key: &str, now: i64) -> R<bool> {
        self.read(|c| {
            let row: Option<String> = c.query_row("SELECT ids FROM context_keys WHERE key = ?1 AND until >= ?2", params![key, now], |r| r.get(0)).optional()?;
            let Some(ids) = row else { return Ok(false) };
            let ids: Vec<String> = serde_json::from_str(&ids).unwrap_or_default();
            if ids.is_empty() {
                return Ok(false);
            }
            let mut revoked = c.prepare_cached("SELECT 1 FROM revoked WHERE id = ?1")?;
            for id in &ids {
                if revoked.exists(params![id])? {
                    return Ok(false);
                }
            }
            Ok(true)
        })
    }

    /// Records what the person approved for `g.actor`, in place of what
    /// they approved for it before; the grant minted under that goes with
    /// it (its caller revokes it).
    pub fn approve_forward(&self, g: &ForwardGrant) -> R<()> {
        let (networks, rights) = (Value::from(g.networks.clone()).to_string(), Value::from(g.rights.clone()).to_string());
        self.write(|c| {
            c.execute(
                "INSERT INTO forward_grants (actor, alpn, key, networks, rights, approved, token, ids, until) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, NULL)
                 ON CONFLICT (actor) DO UPDATE SET alpn = ?2, key = ?3, networks = ?4, rights = ?5, approved = ?6, token = NULL, ids = NULL, until = NULL",
                params![g.actor, g.alpn, g.key, networks, rights, g.approved],
            )?;
            Ok(())
        })
    }

    /// The grant now minted under `actor`'s approval: its first
    /// revocation id, its ids and its end.
    pub fn set_forward_token(&self, actor: &str, token: &str, ids: &[String], until: i64) -> R<()> {
        let ids = Value::from(ids.to_vec()).to_string();
        self.write(|c| {
            c.execute("UPDATE forward_grants SET token = ?2, ids = ?3, until = ?4 WHERE actor = ?1", params![actor, token, ids, until])?;
            Ok(())
        })
    }

    /// What the person approved for `actor`.
    pub fn forward_grant(&self, actor: &str) -> R<Option<ForwardGrant>> {
        Ok(self.forward_grants()?.into_iter().find(|g| g.actor == actor))
    }

    /// Every forward grant the person approved, by actor.
    pub fn forward_grants(&self) -> R<Vec<ForwardGrant>> {
        self.read(|c| {
            let mut q = c.prepare_cached("SELECT actor, alpn, key, networks, rights, approved, token, ids, until FROM forward_grants ORDER BY actor")?;
            let rows = q.query_map([], |r| {
                let token: Option<String> = r.get(6)?;
                let until: Option<i64> = r.get(8)?;
                let ids = list_of(r.get(7)?);
                Ok(ForwardGrant {
                    actor: r.get(0)?,
                    alpn: r.get(1)?,
                    key: r.get(2)?,
                    networks: list_of(r.get(3)?),
                    rights: list_of(r.get(4)?),
                    approved: r.get(5)?,
                    token: token.zip(until).map(|(t, u)| (t, ids, u)),
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    /// Takes back what the person approved for `actor`: what was there.
    pub fn drop_forward_grant(&self, actor: &str) -> R<Option<ForwardGrant>> {
        let had = self.forward_grant(actor)?;
        self.write(|c| {
            c.execute("DELETE FROM forward_grants WHERE actor = ?1", params![actor])?;
            Ok(())
        })?;
        Ok(had)
    }

    /// How the map takes a space folder's marker (`v`, the file's JSON) at
    /// `now`.
    pub fn space_tie(&self, v: &Value, now: i64) -> R<Tie> {
        let Some(m) = marker::verify(v) else { return Ok(Tie::ByName) };
        if !self.context_key(&m.key.id(), now)? {
            return Ok(Tie::ByName);
        }
        let Some(entity) = m.entity() else { return Ok(Tie::ByName) };
        let project = match &entity {
            EntityRef::Name { .. } => return Ok(Tie::Unbound(m)),
            EntityRef::Project(p) => p.clone(),
            _ => return Ok(Tie::Other(m)),
        };
        // The project's, by the person who owns it here, who signed in it.
        let Some(person) = m.person().filter(|_| m.owner_signed).map(str::to_string) else { return Ok(Tie::Other(m)) };
        let Some(id) = self.resolve(&project)? else { return Ok(Tie::Other(m)) };
        match self.project(&id)? {
            Some(p) if p.owner.as_deref() == Some(person.as_str()) => Ok(Tie::Project(m, id)),
            _ => Ok(Tie::Other(m)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{CheckoutKind, Found, Owner as ProjectOwner};
    use std::sync::Arc;
    use xenonet_id::sign::{KeyHome, SeedSigner, Signer};
    use xenonet_log::space::{found, Owner, Space, SpacePolicy};
    use xenonet_model::ids::Ids;

    const WILL: &str = "prs_wwwwwwwwwwwwwwwwwwwwwwwwwwwwwwww";
    const MALLORY: &str = "prs_mmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmm";
    const NOW: i64 = 1_792_512_000_000;

    fn signer(byte: u8) -> SeedSigner {
        SeedSigner::from_seed(&[byte; 32], KeyHome::File).unwrap()
    }

    // A store with one project of Will's, atlas, and its id.
    fn db() -> (Db, ProjectId) {
        let db = Db::memory().unwrap();
        db.use_ids(Ids::seeded("spaces"));
        db.set_owner(Arc::new(|| ProjectOwner { person: Some(WILL.into()), salt: Some([7; 32]) }));
        let atlas = Found {
            path: "/w/atlas".into(),
            path_key: "/w/atlas".into(),
            vcs: Some(".git".into()),
            branch: Some("main".into()),
            head: Some("abc".into()),
            repo_key: Some("github.com/w/atlas".into()),
            common: Some("/w/atlas/.git".into()),
            kind: CheckoutKind::Repo,
            parent: None,
            name: "atlas".into(),
        };
        db.reconcile(&[atlas], 1).unwrap();
        let id = db.checkouts().unwrap()[0].project.clone();
        (db, id)
    }

    // The marker Xenolith writes for a space founded about `about`, owned
    // by `owner` (who signed nothing in it), signed by `context`.
    fn founded(about: EntityRef, owner: Owner, context: &SeedSigner) -> Value {
        let s = Space::found(SpacePolicy::default(), found(about, owner, context, None, "atlas", [1; 16], NOW).unwrap()).unwrap();
        marker::sign(&s, context).unwrap()
    }

    // A marker of a space about `about` that `person` owns and has signed
    // in, as Xenolith signs one: the format's bytes, by `context`.
    fn owned(about: &str, person: &str, context: &SeedSigner) -> Value {
        let v = founded(EntityRef::Name { kind: "project".into(), name: "atlas".into() }, Owner::Person(person.into()), context);
        let text = |k: &str| v[k].as_str().unwrap().to_string();
        marker::sign_parts(&text("space"), about, &text("genesis"), &format!("person:{person}"), true, context).unwrap()
    }

    // Xenolith's grant here, naming its context key.
    fn grant(db: &Db, context: &SeedSigner) {
        db.set_context_key(&context.public().id(), "xenolith", &["aa01".into()], NOW + 60_000, NOW).unwrap();
    }

    #[test]
    fn a_valid_marker_ties_the_space_to_its_project_by_id() {
        let (db, atlas) = db();
        let lith = signer(1);
        grant(&db, &lith);
        let m = owned(atlas.as_str(), WILL, &lith);
        match db.space_tie(&m, NOW).unwrap() {
            Tie::Project(_, p) => assert_eq!(p, atlas),
            t => panic!("{t:?}"),
        }
        let v = db.space_tie(&m, NOW).unwrap().to_json();
        assert_eq!((v["tie"].as_str(), v["project"].as_str()), (Some("id"), Some(atlas.as_str())));
    }

    #[test]
    fn an_edited_unsigned_or_strangers_marker_leaves_the_space_by_name() {
        let (db, atlas) = db();
        let (lith, agent) = (signer(1), signer(2));
        grant(&db, &lith);
        let m = owned(atlas.as_str(), WILL, &lith);
        let mut edited = owned("prj_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", WILL, &lith);
        edited["about"] = json!(atlas.as_str());
        assert_eq!(db.space_tie(&edited, NOW).unwrap(), Tie::ByName, "edited: its signature no longer holds");
        let mut unsigned = m.clone();
        unsigned.as_object_mut().unwrap().remove("sig");
        assert_eq!(db.space_tie(&unsigned, NOW).unwrap(), Tie::ByName);
        assert_eq!(db.space_tie(&json!({ "by": "hand" }), NOW).unwrap(), Tie::ByName);
        // Signed whole, but by a key no grant here names: an agent's.
        assert_eq!(db.space_tie(&owned(atlas.as_str(), WILL, &agent), NOW).unwrap(), Tie::ByName);
        // Xenolith's key past its grant's time, or with the grant revoked.
        assert_eq!(db.space_tie(&m, NOW + 60_001).unwrap(), Tie::ByName);
        db.write(|c| Ok(c.execute("INSERT INTO revoked (id, actor, reason, at) VALUES ('aa01', 'xenolith', 'revoked', 1)", [])?)).unwrap();
        assert_eq!(db.space_tie(&m, NOW).unwrap(), Tie::ByName);
    }

    #[test]
    fn a_space_about_the_project_is_its_only_when_its_owner_owns_it_and_signed() {
        let (db, atlas) = db();
        let lith = signer(1);
        grant(&db, &lith);
        // Owned by someone else: about atlas, owned by Mallory.
        let t = db.space_tie(&owned(atlas.as_str(), MALLORY, &lith), NOW).unwrap();
        assert!(matches!(t, Tie::Other(_)), "{t:?}");
        let v = t.to_json();
        assert_eq!((v["tie"].as_str(), v["about"].as_str(), v["owner"].as_str()), (Some("none"), Some(atlas.as_str()), Some(format!("person:{MALLORY}").as_str())));
        // Named Will's by its founder, signed by nobody of Will's.
        let named = founded(EntityRef::Project(atlas.clone()), Owner::Person(WILL.into()), &lith);
        assert!(matches!(db.space_tie(&named, NOW).unwrap(), Tie::Other(_)));
        // About a project this machine doesn't hold.
        assert!(matches!(db.space_tie(&owned("prj_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", WILL, &lith), NOW).unwrap(), Tie::Other(_)));
        // Not bound yet: by its name.
        let unbound = founded(EntityRef::Name { kind: "project".into(), name: "atlas".into() }, Owner::Person(WILL.into()), &lith);
        assert!(matches!(db.space_tie(&unbound, NOW).unwrap(), Tie::Unbound(_)));
    }

    #[test]
    fn a_forward_grant_is_approved_minted_under_and_taken_back() {
        let (db, _) = db();
        let g = ForwardGrant {
            actor: "xenolith".into(),
            alpn: "xenolith/sync/2".into(),
            key: signer(1).public().id(),
            networks: vec!["net_00000000000000000000000000000001".into()],
            rights: vec!["ctx-read".into()],
            approved: NOW,
            token: None,
        };
        db.approve_forward(&g).unwrap();
        assert_eq!(db.forward_grant("xenolith").unwrap(), Some(g.clone()));
        db.set_forward_token("xenolith", "aa01", &["aa01".into(), "aa02".into()], NOW + 5).unwrap();
        let minted = db.forward_grant("xenolith").unwrap().unwrap();
        assert_eq!(minted.token, Some(("aa01".into(), vec!["aa01".into(), "aa02".into()], NOW + 5)));
        assert_eq!(minted.to_json()["until"], NOW + 5);
        assert!(!minted.to_json().to_string().contains("token"), "no token's text or id in what the window lists");
        // Approved again (other networks): what was minted under the old goes.
        db.approve_forward(&ForwardGrant { networks: vec![], ..g.clone() }).unwrap();
        assert_eq!(db.forward_grant("xenolith").unwrap().unwrap().token, None);
        assert_eq!(db.drop_forward_grant("xenolith").unwrap().map(|g| g.actor), Some("xenolith".into()));
        assert_eq!(db.forward_grants().unwrap(), vec![]);
    }

    #[test]
    fn a_later_grant_naming_the_key_replaces_the_earlier() {
        let (db, _) = db();
        let lith = signer(1);
        grant(&db, &lith);
        db.set_context_key(&lith.public().id(), "xenolith", &["bb02".into()], NOW + 120_000, NOW + 1).unwrap();
        assert!(db.context_key(&lith.public().id(), NOW + 90_000).unwrap());
        db.write(|c| Ok(c.execute("INSERT INTO revoked (id, actor, reason, at) VALUES ('aa01', 'xenolith', 'revoked', 1)", [])?)).unwrap();
        assert!(db.context_key(&lith.public().id(), NOW).unwrap(), "the old grant's revocation isn't the new one's");
        assert!(!db.context_key(&signer(2).public().id(), NOW).unwrap());
    }
}
