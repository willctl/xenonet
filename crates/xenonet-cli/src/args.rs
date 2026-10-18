//! The xenonet command's arguments (cli/lib/args.js): flags (--flag,
//! --flag=value, --flag value for the flags that take one, --no-flag, -h,
//! -v) anywhere, and the rest in order. Everything after -- is taken as it
//! is. A flag that may come more than once (xenonet actor targets --add …
//! --add …) collects its values.
//!
//! Flags keep the values JS gives them, so the commands read them as the JS
//! does: true, false, a string, or a list of strings for the ones that
//! repeat, under the flag's name in camelCase.

use serde_json::{Map, Value};
use xenonet_base::client::Pick;
use xenonet_base::text::{js_number, js_string, truthy};

const TAKES_VALUE: &[&str] = &[
    "port",
    "only",
    "limit",
    "dir",
    "instance",
    "seconds",
    "theme",
    "size",
    "out",
    "last",
    "since",
    // xenonet hosts
    "sshPort",
    "xenonetPort",
    "identity",
    "knownHosts",
    "hostKeyAlias",
    "command",
    // xenonet actor
    "name",
    "slug",
    "capabilities",
    "heartbeat",
    "product",
    "tokenFile",
    "presence",
    "status",
    "cwd",
    "pid",
    "target",
    "path",
    "count",
    "paths",
    "platform",
    "subject",
    "ref",
    "app",
    "add",
    "remove",
    "wait",
    "replyTo",
    // xenonet mcp assistant, xenonet assistant
    "grant",
    // xenonet gateway
    "tunnel",
    "host",
    // xenonet actor listen
    "exec",
];
const REPEATS: &[&str] = &["add", "remove", "paths"];

/// The flags given, by name: what JS's `flags` object holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flags(pub Map<String, Value>);

impl Flags {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn set(&mut self, key: &str, value: Value) {
        self.0.insert(key.to_string(), value);
    }

    /// `!!flags.key`
    pub fn on(&self, key: &str) -> bool {
        truthy(self.get(key))
    }

    /// `flags.key === true`
    pub fn is_true(&self, key: &str) -> bool {
        self.get(key) == Some(&Value::Bool(true))
    }

    /// `flags.key === false`
    pub fn is_false(&self, key: &str) -> bool {
        self.get(key) == Some(&Value::Bool(false))
    }

    /// `flags.key !== undefined`
    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// The value when `typeof flags.key === "string"`.
    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// `flags.key || null`, as text: what a template string makes of it.
    pub fn text(&self, key: &str) -> Option<String> {
        self.get(key).filter(|v| truthy(Some(v))).map(js_string)
    }

    /// `Number(flags.key)`, NaN when it isn't there.
    pub fn number(&self, key: &str) -> f64 {
        js_number(self.get(key))
    }

    /// `[].concat(flags.key ?? [])`, as text.
    pub fn list(&self, key: &str) -> Vec<String> {
        match self.get(key) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(a)) => a.iter().map(js_string).collect(),
            Some(v) => vec![js_string(v)],
        }
    }

    /// Which XenoNet the flags name, for core/client.js's candidates():
    /// --port (when it's anything), --demo, --dev, and XENONET_PORT.
    pub fn pick(&self) -> Pick {
        Pick {
            port: self.on("port").then(|| self.number("port")),
            demo: self.on("demo"),
            dev: self.on("dev"),
            env_port: std::env::var("XENONET_PORT").ok().filter(|p| !p.is_empty()),
        }
    }
}

/// What `parse` found: the flags, and the other words in order.
#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub flags: Flags,
    pub rest: Vec<String>,
}

// name.replace(/-([a-z])/g, (_, ch) => ch.toUpperCase())
fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut chars = name.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '-' {
            if let Some(&n) = chars.peek().filter(|n| n.is_ascii_lowercase()) {
                out.push(n.to_ascii_uppercase());
                chars.next();
                continue;
            }
        }
        out.push(c);
    }
    out
}

pub fn parse(argv: &[String]) -> Parsed {
    let mut flags = Flags::default();
    let mut rest = Vec::new();
    let set = |flags: &mut Flags, key: &str, value: String| {
        if REPEATS.contains(&key) {
            let mut list = match flags.get(key) {
                Some(Value::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            list.push(Value::String(value));
            flags.set(key, Value::Array(list));
        } else {
            flags.set(key, Value::String(value));
        }
    };
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        if a == "--" {
            rest.extend(argv[i + 1..].iter().cloned());
            break;
        }
        if a == "-h" {
            flags.set("help", Value::Bool(true));
        } else if a == "-v" {
            flags.set("version", Value::Bool(true));
        } else if let Some(body) = a.strip_prefix("--") {
            // split(/=(.*)/s): the name, and everything after the first =.
            let (k, v) = match body.split_once('=') {
                Some((k, v)) => (k, Some(v)),
                None => (body, None),
            };
            let key = camel(k);
            if let Some(v) = v {
                set(&mut flags, &key, v.to_string());
            } else if TAKES_VALUE.contains(&key.as_str()) && i + 1 < argv.len() {
                i += 1;
                set(&mut flags, &key, argv[i].clone());
            } else if let Some(name) = k.strip_prefix("no-") {
                flags.set(&camel(name), Value::Bool(false));
            } else {
                flags.set(&key, Value::Bool(true));
            }
        } else {
            rest.push(a.clone());
        }
        i += 1;
    }
    Parsed { flags, rest }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(ToString::to_string).collect()
    }

    fn flags(v: Value) -> Flags {
        Flags(v.as_object().unwrap().clone())
    }

    // The same cases as test/cli.test.js and test/cli-actor.test.js.
    #[test]
    fn flags_anywhere_values_negations_and_everything_after_dashes() {
        assert_eq!(
            parse(&args(&["doctor", "--only", "claude,codex", "--no-color", "--port=47842", "-h"])),
            Parsed { flags: flags(json!({ "only": "claude,codex", "color": false, "port": "47842", "help": true })), rest: args(&["doctor"]) }
        );
        assert_eq!(parse(&args(&["who", "--json", "src/a.js"])), Parsed { flags: flags(json!({ "json": true })), rest: args(&["who", "src/a.js"]) });
        assert_eq!(parse(&args(&["console", "--instance", "dev", "--", "type:--x"])), Parsed { flags: flags(json!({ "instance": "dev" })), rest: args(&["console", "type:--x"]) });
        assert_eq!(parse(&args(&["--dry-run"])).flags, flags(json!({ "dryRun": true })));
    }

    #[test]
    fn values_and_repeats() {
        let p = parse(&args(&["actor", "targets", "--add", "/a=watches", "--add", "claude:x=spawned", "--remove", "/b", "--replace"]));
        assert_eq!((p.rest, p.flags), (args(&["actor", "targets"]), flags(json!({ "add": ["/a=watches", "claude:x=spawned"], "remove": ["/b"], "replace": true }))));
        assert_eq!(
            parse(&args(&["actor", "heartbeat", "--status", "", "--presence", "idle", "--token-file=x"])).flags,
            flags(json!({ "status": "", "presence": "idle", "tokenFile": "x" }))
        );
        assert_eq!(
            parse(&args(&["actor", "event", "edited", "--paths", "a.ts", "--paths", "b.ts", "--count", "2"])).flags,
            flags(json!({ "paths": ["a.ts", "b.ts"], "count": "2" }))
        );
        // A flag that takes a value, last, has none: it's on.
        assert_eq!(parse(&args(&["agents", "--port"])).flags, flags(json!({ "port": true })));
        // Everything after the first =, and a negation's name in camelCase.
        assert_eq!(parse(&args(&["--a=b=c", "--no-exit-code", "-v"])).flags, flags(json!({ "a": "b=c", "exitCode": false, "version": true })));
        assert_eq!(camel("host-key-alias"), "hostKeyAlias");
        assert_eq!(camel("a-B-c-"), "a-BC-");
    }

    #[test]
    fn which_xenonet() {
        let f = parse(&args(&["--port", "5000", "--dev"])).flags;
        let p = f.pick();
        assert_eq!((p.port, p.dev, p.demo), (Some(5000.0), true, false));
        // --port with nothing after it is Number(true), as JS has it.
        assert_eq!(parse(&args(&["--port"])).flags.pick().port, Some(1.0));
        assert_eq!(parse(&args(&["--port="])).flags.pick().port, None);
    }
}
