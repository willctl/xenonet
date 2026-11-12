//! Text that leaves this machine with the home folder written as ~: in
//! every string and key, in each form the folder takes here, and the name
//! of any other folder named for a person that the agents work in. What
//! presence sends of an agent (its folder, its question, its last message,
//! why it seems stuck) goes through it.

use serde_json::{Map, Value};
use xenonet_os::paths::path_key;

fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

// One character as /i compares them.
fn fold(c: char) -> char {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

// Where `form` matches `s` at byte `at`, case aside: the byte it ends at.
fn starts(s: &str, at: usize, form: &str) -> Option<usize> {
    let mut it = s[at..].char_indices();
    for f in form.chars() {
        let (_, c) = it.next()?;
        if fold(c) != fold(f) {
            return None;
        }
    }
    Some(it.next().map_or(s.len(), |(i, _)| at + i))
}

/// One of the scrubber's swaps: what it matches, and what may not follow.
enum Swap {
    /// Any of `forms` (in order), not followed by a stop: becomes ~.
    Whole { forms: Vec<String>, stops: fn(char) -> bool },
    /// One of `parents`, then `sep` (a separator, or none), then `name`,
    /// not followed by a stop: the parent part stays, the name becomes ~.
    Named { parents: Vec<String>, seps: &'static [char], name: String, stops: fn(char) -> bool },
}

fn stop_wdd(c: char) -> bool {
    word(c) || c == '.' || c == '-'
}

fn stop_wd(c: char) -> bool {
    word(c) || c == '.'
}

impl Swap {
    // The match at `at`: (where it ends, what it becomes).
    fn at(&self, s: &str, at: usize) -> Option<(usize, String)> {
        let free = |end: usize, stops: fn(char) -> bool| s[end..].chars().next().is_none_or(|c| !stops(c));
        match self {
            Swap::Whole { forms, stops } => forms.iter().find_map(|f| starts(s, at, f).filter(|e| free(*e, *stops)).map(|e| (e, "~".to_string()))),
            Swap::Named { parents, seps, name, stops } => parents.iter().find_map(|p| {
                let mut e = starts(s, at, p)?;
                if !seps.is_empty() {
                    let c = s[e..].chars().next()?;
                    if !seps.contains(&c) {
                        return None;
                    }
                    e += c.len_utf8();
                }
                let kept = s[at..e].to_string();
                let end = starts(s, e, name)?;
                free(end, *stops).then(|| (end, format!("{kept}~")))
            }),
        }
    }

    fn replace(&self, s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut i = 0;
        while let Some(c) = s[i..].chars().next() {
            if let Some((end, to)) = self.at(s, i) {
                out.push_str(&to);
                if end == i {
                    out.push(c);
                    i += c.len_utf8();
                } else {
                    i = end;
                }
                continue;
            }
            out.push(c);
            i += c.len_utf8();
        }
        out
    }
}

/// A copy of a value with the home folder written as ~ in every string and
/// every key, in each form it takes here: as XenoNet was given it, as a
/// path key, and as Claude Code names its project folders
/// (-Users-nova-Projects-atlas). `also` lists other folders named for a
/// person, which go with ~ in place of the name alone.
pub struct Scrubber {
    swaps: Vec<Swap>,
    shortest: usize,
}

fn dashed(s: &str) -> String {
    s.chars().map(|c| if matches!(c, '\\' | '/' | '.' | ':') { '-' } else { c }).collect()
}

fn unique(items: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for i in items {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

impl Scrubber {
    pub fn new(home: Option<&str>, also: &[String]) -> Self {
        let h = home.unwrap_or("").trim_end_matches(['\\', '/']).to_string();
        let mut swaps = Vec::new();
        let mut lengths = Vec::new();
        if xenonet_text::utf16::len(&h) >= 2 {
            let mut forms = vec![h.clone(), h.replace('\\', "/")];
            if let Some(k) = path_key(&h) {
                forms.push(k);
            }
            let forms = unique(forms);
            lengths.extend(forms.iter().map(|f| xenonet_text::utf16::len(f)));
            swaps.push(Swap::Whole { forms, stops: stop_wdd });
            swaps.push(Swap::Whole { forms: vec![dashed(&h)], stops: stop_wd });
        }
        for folder in also {
            let f = folder.trim_end_matches(['\\', '/']);
            let cut = f.rfind(['/', '\\']);
            let Some(cut) = cut.filter(|c| *c >= 1 && *c != f.len() - 1) else { continue };
            let (parent, name) = (&f[..cut], &f[cut + 1..]);
            let parents = unique(vec![parent.to_string(), parent.replace('\\', "/")]);
            swaps.push(Swap::Named { parents, seps: &['\\', '/'], name: name.to_string(), stops: stop_wdd });
            swaps.push(Swap::Named { parents: vec![format!("{}-", dashed(parent))], seps: &[], name: name.to_string(), stops: stop_wd });
            lengths.push(xenonet_text::utf16::len(f));
        }
        let shortest = if swaps.is_empty() { usize::MAX } else { lengths.into_iter().min().unwrap_or(usize::MAX) };
        Scrubber { swaps, shortest }
    }

    pub fn text(&self, s: &str) -> String {
        if xenonet_text::utf16::len(s) < self.shortest {
            return s.to_string();
        }
        self.swaps.iter().fold(s.to_string(), |t, sw| sw.replace(&t))
    }

    pub fn walk(&self, v: &Value) -> Value {
        match v {
            Value::String(s) => Value::String(self.text(s)),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.walk(x)).collect()),
            Value::Object(m) => {
                let mut out = Map::new();
                for (k, x) in m {
                    out.insert(self.text(k), self.walk(x));
                }
                Value::Object(out)
            }
            other => other.clone(),
        }
    }
}

/// The folder named for a person that a path is in, when it isn't the home
/// folder: `/Users/<name>` or `/home/<name>`, after a drive if there is one
/// (C:\Users\nova, /c/Users/nova, or WSL's /mnt/c/Users/nova).
pub fn user_folder_of(p: Option<&Value>, home: Option<&str>) -> Option<String> {
    let s = p.and_then(Value::as_str).unwrap_or("");
    let found = user_folder(s)?;
    let key = path_key(&found).unwrap_or_default();
    let home_key = home.and_then(path_key).filter(|k| !k.is_empty());
    match home_key {
        Some(h) if key == h || key.starts_with(&format!("{h}/")) => None,
        _ => Some(found),
    }
}

fn sep(c: Option<char>) -> bool {
    matches!(c, Some('/' | '\\'))
}

// ^((?:[A-Za-z]:|\/mnt\/[A-Za-z]|\/[A-Za-z](?=[\\/]))?[\\/](?:Users|home)[\\/][^\\/]+)(?=[\\/]|$)/i
fn user_folder(s: &str) -> Option<String> {
    let b: Vec<char> = s.chars().collect();
    let at = |i: usize| b.get(i).copied();
    let letter = |i: usize| at(i).is_some_and(|c| c.is_ascii_alphabetic());
    let mut prefixes = Vec::new();
    if letter(0) && at(1) == Some(':') {
        prefixes.push(2);
    }
    if at(0) == Some('/') && starts(s, 1, "mnt/").is_some() && letter(5) {
        prefixes.push(6);
    }
    if at(0) == Some('/') && letter(1) && sep(at(2)) {
        prefixes.push(2);
    }
    prefixes.push(0);
    for p in prefixes {
        if !sep(at(p)) {
            continue;
        }
        let rest: String = b[p + 1..].iter().collect();
        let word_len = ["Users", "home"].iter().find_map(|w| starts(&rest, 0, w).map(|_| w.chars().count()));
        let Some(w) = word_len else { continue };
        let i = p + 1 + w;
        if !sep(at(i)) {
            continue;
        }
        let mut j = i + 1;
        while j < b.len() && !sep(at(j)) {
            j += 1;
        }
        if j == i + 1 {
            continue;
        }
        return Some(b[..j].iter().collect());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_home_folder_goes_as_tilde() {
        let s = Scrubber::new(Some("/Users/nova"), &["/mnt/c/Users/nova2".into()]);
        assert_eq!(s.text("/Users/nova/Projects/atlas"), "~/Projects/atlas");
        assert_eq!(s.text("/USERS/NOVA/x"), "~/x");
        assert_eq!(s.text("/Users/novak/x"), "/Users/novak/x", "another name that starts the same stays");
        assert_eq!(s.text("-Users-nova-Projects-atlas"), "~-Projects-atlas");
        assert_eq!(s.text("/mnt/c/Users/nova2/Projects"), "/mnt/c/Users/~/Projects");
        assert_eq!(s.walk(&json!({ "/Users/nova/x": ["/Users/nova"] })), json!({ "~/x": ["~"] }));
        assert_eq!(user_folder_of(Some(&json!("/mnt/c/Users/nova/Projects")), Some("/home/nova")).as_deref(), Some("/mnt/c/Users/nova"));
        assert_eq!(user_folder_of(Some(&json!("/home/nova/Projects")), Some("/home/nova")), None);
        assert_eq!(user_folder_of(Some(&json!("C:\\Users\\nova\\x")), None).as_deref(), Some("C:\\Users\\nova"));
        assert_eq!(user_folder_of(Some(&json!("/c/Users/nova")), None).as_deref(), Some("/c/Users/nova"));
    }
}
