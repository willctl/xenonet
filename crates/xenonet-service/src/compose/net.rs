//! What this machine keeps of its network and its person
//! (`xenonet_net::persist::Persist`), in the store: the network's row, its
//! roster and each person's chain as chain entries, how the link stood and
//! where it stands in the gateway's stream, the services published, the
//! presence blocks taken and the token roots' certificates
//! (`xenonet_store::net`), and this machine's feed and how it stands with
//! the others' (`xenonet_store::feed`). Here at the composition root,
//! which knows both: the network depends on no store, and the store on no
//! network.

use serde_json::Value;
use std::sync::Arc;
use xenonet_id::codec::{b64, unb64_n};
use xenonet_net::feed::{FeedEntry, Fork, Pos};
use xenonet_net::files::{NetFile, Pin};
use xenonet_net::holder::Held;
use xenonet_net::identity::person::{self, PersonPolicy};
use xenonet_net::mailbox::Mail;
use xenonet_net::persist::{Persist, Person, Seen, WriterAt};
use xenonet_net::roster::{self, NetworkPolicy};
use xenonet_net::share::Sharing;
use xenonet_net::{NetError, R};
use xenonet_store::net::{self as kept, Gateway, Network};
use xenonet_store::{Db, StoreError};

/// The store, as the network and the person keep in it.
pub struct StorePersist(pub Arc<Db>);

fn store(e: StoreError) -> NetError {
    NetError::internal(format!("the store: {}", e.message))
}

fn network_of(n: &NetFile) -> Network {
    let g = &n.gateway;
    Network {
        id: n.id.clone(),
        name: n.name.clone(),
        host: n.host.clone(),
        founder: n.founder.clone(),
        gateway: Gateway { host: g.host.clone(), key_id: g.key_id.clone(), sign: g.sign.clone(), seal: g.seal.clone(), address: g.address.clone() },
        lan: n.lan.clone(),
        titles_only: n.titles_only,
        share_load: n.share_load,
        joined: n.joined,
    }
}

fn net_file(n: Network) -> NetFile {
    let g = n.gateway;
    NetFile {
        id: n.id,
        name: n.name,
        host: n.host,
        founder: n.founder,
        gateway: Pin { host: g.host, key_id: g.key_id, sign: g.sign, seal: g.seal, address: g.address },
        lan: n.lan,
        titles_only: n.titles_only,
        share_load: n.share_load,
        joined: n.joined,
    }
}

impl Persist for StorePersist {
    fn networks(&self) -> R<Vec<NetFile>> {
        Ok(self.0.networks().map_err(store)?.into_iter().map(net_file).collect())
    }
    fn put_network(&self, n: &NetFile) -> R<()> {
        self.0.put_network(&network_of(n)).map_err(store)
    }
    fn roster(&self, network: &str) -> R<Vec<roster::Entry>> {
        self.0.roster::<NetworkPolicy>(network).map_err(store)
    }
    fn put_roster(&self, network: &str, entries: &[roster::Entry]) -> R<()> {
        self.0.put_roster(network, entries).map_err(store)
    }
    fn link(&self, network: &str) -> R<Option<Value>> {
        self.0.link_status(network).map_err(store)
    }
    fn put_link(&self, network: &str, status: &Value) -> R<()> {
        self.0.put_link_status(network, status).map_err(store)
    }
    fn services(&self, network: &str) -> R<Vec<Value>> {
        self.0.services(network).map_err(store)
    }
    fn put_services(&self, network: &str, services: &[Value]) -> R<()> {
        self.0.put_services(network, services).map_err(store)
    }
    fn forget(&self, network: &str) -> R<()> {
        self.0.forget_network(network).map_err(store)
    }
    fn persons(&self) -> R<Vec<Person>> {
        let mut out = Vec::new();
        for p in self.0.persons().map_err(store)? {
            let entries: Vec<person::Entry> = self.0.person::<PersonPolicy>(&p.id).map_err(store)?;
            out.push((p.id, p.own, entries));
        }
        Ok(out)
    }
    fn put_person(&self, id: &str, own: bool, entries: &[person::Entry]) -> R<()> {
        self.0.put_person(id, own, entries).map_err(store)
    }
    fn forget_others(&self) -> R<()> {
        self.0.forget_others().map_err(store)
    }
    fn presence_seen(&self) -> R<Vec<Seen>> {
        // A nonce that doesn't read is dropped, as a block it can't check.
        Ok(self.0.presence_seen().map_err(store)?.into_iter().filter_map(|s| Some((s.device, unb64_n::<16>(&s.nonce)?, s.at))).collect())
    }
    fn put_presence_seen(&self, seen: &[Seen]) -> R<()> {
        let rows: Vec<kept::Seen> = seen.iter().map(|(device, nonce, at)| kept::Seen { device: device.clone(), nonce: b64(nonce), at: *at }).collect();
        self.0.put_presence_seen(&rows).map_err(store)
    }
    fn token_roots(&self) -> R<Vec<Value>> {
        self.0.token_roots().map_err(store)
    }
    fn put_token_roots(&self, certificates: &[Value]) -> R<()> {
        self.0.put_token_roots(certificates).map_err(store)
    }
    fn cursor(&self, network: &str) -> R<Option<(String, u64)>> {
        self.0.link_cursor(network).map_err(store)
    }
    fn put_cursor(&self, network: &str, epoch: &str, seq: u64) -> R<()> {
        self.0.put_link_cursor(network, epoch, seq).map_err(store)
    }
    fn feed_writer(&self, network: &str) -> R<Option<WriterAt>> {
        Ok(self.0.feed_writer(network).map_err(store)?.map(|w| WriterAt { inc: w.inc, kept: w.kept, loose: w.loose, link: w.link, head: w.head }))
    }
    fn put_feed_writer(&self, network: &str, author: &str, at: WriterAt, kept: Option<&FeedEntry>) -> R<()> {
        let at = xenonet_store::feed::WriterAt { inc: at.inc, kept: at.kept, loose: at.loose, link: at.link, head: at.head };
        self.0.put_feed_writer(network, author, at, kept).map_err(store)
    }
    fn feed_out_after(&self, network: &str, after: Pos, limit: usize) -> R<Vec<FeedEntry>> {
        self.0.feed_out_after(network, after, limit).map_err(store)
    }
    fn feed_ack(&self, network: &str, member: &str) -> R<Option<Pos>> {
        self.0.feed_ack(network, member).map_err(store)
    }
    fn put_feed_ack(&self, network: &str, member: &str, at: Pos) -> R<()> {
        self.0.put_feed_ack(network, member, at).map_err(store)
    }
    fn sharing(&self, network: &str) -> R<Option<Sharing>> {
        // One that doesn't read is taken as said by nobody: the default.
        Ok(self.0.sharing(network).map_err(store)?.as_ref().and_then(Sharing::parse))
    }
    fn put_sharing(&self, network: &str, sharing: &Sharing) -> R<()> {
        self.0.put_sharing(network, &sharing.to_json()).map_err(store)
    }
    fn own_relays(&self) -> R<Vec<String>> {
        self.0.own_relays().map_err(store)
    }
    fn put_own_relays(&self, urls: &[String]) -> R<()> {
        self.0.put_own_relays(urls).map_err(store)
    }
    fn feed_heads(&self, network: &str) -> R<Vec<(String, Pos)>> {
        self.0.feed_heads(network).map_err(store)
    }
    fn frozen_feeds(&self, network: &str) -> R<Vec<(String, Fork)>> {
        Ok(self.0.frozen_feeds(network).map_err(store)?.into_iter().map(|f| (f.author, f.fork)).collect())
    }
    fn held(&self, network: &str, author: &str, from: Pos, limit: usize) -> R<Vec<Held>> {
        Ok(self.0.feed_in_from(author, network, from, limit).map_err(store)?.into_iter().map(|(entry, open)| Held { entry, open }).collect())
    }
    fn put_mail(&self, m: &Mail, evict: usize) -> R<bool> {
        let kept = xenonet_store::mailbox::Mail {
            network: m.network.clone(),
            recipient: m.recipient.clone(),
            author: m.author.clone(),
            id: m.id.clone(),
            kept: m.kept,
            envelope: m.envelope.clone(),
        };
        self.0.put_mail(&kept, evict).map_err(store)
    }
    fn mail_for(&self, network: &str, recipient: &str, limit: usize) -> R<Vec<Mail>> {
        let rows = self.0.mail_for(network, recipient, limit).map_err(store)?;
        Ok(rows.into_iter().map(|m| Mail { network: m.network, recipient: m.recipient, author: m.author, id: m.id, kept: m.kept, envelope: m.envelope }).collect())
    }
    fn mail_sizes(&self, network: &str, recipient: &str) -> R<Vec<u64>> {
        self.0.mail_sizes(network, recipient).map_err(store)
    }
    fn mail_took(&self, network: &str, recipient: &str, ids: &[String]) -> R<()> {
        self.0.mail_took(network, recipient, ids).map_err(store)
    }
    fn prune_mail(&self, before: u64) -> R<u64> {
        self.0.prune_mail(before).map_err(store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_network_and_its_person_are_kept_across_a_restart() {
        let dir = std::env::temp_dir().join(format!("xenonet-service-persist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let open = || -> Arc<dyn Persist> { Arc::new(StorePersist(Arc::new(Db::open(&dir).unwrap()))) };
        let kept = open();
        let identity = xenonet_net::identity::Identity::open(dir.join("identity"), kept.clone(), Some(Arc::new(xenonet_keys::Fake { allow: true }))).unwrap();
        identity.found(true, "studio", 1).unwrap();
        let pin = Pin { host: "studio".into(), key_id: "k".into(), sign: "s".into(), seal: "x".into(), address: Some("127.0.0.1:1".into()) };
        let n = NetFile {
            id: "net_0".into(),
            name: "home".into(),
            host: "studio".into(),
            founder: "studio".into(),
            gateway: pin,
            lan: None,
            titles_only: true,
            share_load: false,
            joined: 7,
        };
        kept.put_network(&n).unwrap();
        kept.put_presence_seen(&[("xns1_a".into(), [3; 16], 9)]).unwrap();
        drop((kept, identity));
        let again = open();
        assert_eq!(again.network().unwrap(), Some(n));
        assert_eq!(again.presence_seen().unwrap(), [("xns1_a".to_string(), [3; 16], 9)]);
        let back = xenonet_net::identity::Identity::open(dir.join("identity"), again.clone(), None).unwrap();
        assert!(back.person_id().is_some(), "the person's chain is kept");
        again.forget("net_0").unwrap();
        assert_eq!(again.network().unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
