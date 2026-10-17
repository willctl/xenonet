//! A map with a JS Map's order: keys in the order they were first added
//! (setting one that's there keeps its place, and one deleted and added
//! again goes last), and a walk over it that, like a for…of over a Map,
//! sees keys added while it goes and skips keys deleted before it gets to
//! them. The blocks keep their files and folders this way, because the
//! order they read them in is the order things happen.

use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug)]
pub struct OrderedMap<V> {
    by_key: HashMap<String, (u64, V)>,
    order: BTreeMap<u64, String>,
    next: u64,
}

impl<V> Default for OrderedMap<V> {
    fn default() -> Self {
        OrderedMap { by_key: HashMap::new(), order: BTreeMap::new(), next: 0 }
    }
}

/// Where a walk over the map is: after the key at this place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor(Option<u64>);

impl Cursor {
    /// A cursor from where `at` says, as a runtime keeps it between steps.
    pub fn of(at: Option<u64>) -> Self {
        Cursor(at)
    }
    pub fn at(self) -> Option<u64> {
        self.0
    }
}

impl<V> OrderedMap<V> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, key: &str, value: V) {
        match self.by_key.get_mut(key) {
            Some(slot) => slot.1 = value,
            None => {
                let at = self.next;
                self.next += 1;
                self.order.insert(at, key.to_string());
                self.by_key.insert(key.to_string(), (at, value));
            }
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<V> {
        let (at, v) = self.by_key.remove(key)?;
        self.order.remove(&at);
        Some(v)
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.by_key.get(key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.by_key.get_mut(key).map(|(_, v)| v)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.by_key.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    pub fn clear(&mut self) {
        self.by_key.clear();
        self.order.clear();
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.order.values().map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &V)> {
        self.order.values().map(|k| (k.as_str(), &self.by_key[k].1))
    }

    /// Keeps only what `keep` says to, in order.
    pub fn retain(&mut self, mut keep: impl FnMut(&str, &V) -> bool) {
        let gone: Vec<String> = self.iter().filter(|(k, v)| !keep(k, v)).map(|(k, _)| k.to_string()).collect();
        for k in gone {
            self.remove(&k);
        }
    }

    /// The next key of a walk, and where the walk is then.
    pub fn next_after(&self, cursor: Cursor) -> Option<(Cursor, &str)> {
        let from = cursor.0.map_or(0, |c| c + 1);
        self.order.range(from..).next().map(|(at, k)| (Cursor(Some(*at)), k.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_js_maps_order() {
        let mut m = OrderedMap::new();
        for k in ["b", "a", "c"] {
            m.insert(k, 0);
        }
        m.insert("b", 1);
        m.remove("a");
        m.insert("a", 2);
        assert_eq!(m.keys().collect::<Vec<_>>(), ["b", "c", "a"]);
        // A walk sees what's added on the way, and not what's deleted ahead of it.
        let (c, first) = m.next_after(Cursor::default()).map(|(c, k)| (c, k.to_string())).unwrap();
        assert_eq!(first, "b");
        m.remove("c");
        m.insert("d", 3);
        let mut seen = vec![first];
        let mut at = c;
        while let Some((next, k)) = m.next_after(at) {
            seen.push(k.to_string());
            at = next;
        }
        assert_eq!(seen, ["b", "a", "d"]);
    }
}
