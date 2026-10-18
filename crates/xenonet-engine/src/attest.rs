//! How XenoNet knows who sent a message (docs/guide/agents.md, Messages):
//! an envelope, message/envelope/v1, that XenoNet writes from what its own
//! checks found when the message came in, kept apart from what the sender
//! said: core/engine/attest.js. The sender's words can't write into it:
//! it's built from the route's verdict, never from the request's body.
//!
//!   sender     { id, kind: agent|actor|you, name, product }
//!   auth       how it proved who it is: session, token, window, grant or
//!              gateway
//!   process    what XenoNet saw of the program that sent it: signed,
//!              checked, unchecked, or n/a
//!   strength   strong, partial or weak: how far the checks go
//!   client     the program it came from, when XenoNet saw it (home as ~)
//!   line       the same in a sentence, for people and agents
//!
//! It says what XenoNet checked and names what it couldn't; it's never an
//! approval of anything the message asks.

use crate::peer::Thrown;
use crate::sql::Arg;
use crate::store::js_text;

pub const SCHEMA: &str = "message/envelope/v1";

fn process_of(verdict: &Arg, method: &str) -> &'static str {
    if method == "window" || method == "gateway" {
        return "n/a";
    }
    if !verdict.field("checked").truthy() {
        return "unchecked";
    }
    if verdict.field("sure").truthy() {
        "signed"
    } else {
        "checked"
    }
}

fn strength_of(method: &str, process: &str) -> &'static str {
    match method {
        "window" => "strong",
        "session" => match process {
            "signed" => "strong",
            "checked" => "partial",
            _ => "weak",
        },
        "token" | "grant" if matches!(process, "checked" | "signed") => "partial",
        _ => "weak",
    }
}

/// path.basename on this machine.
fn basename(p: &str) -> String {
    let seps: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let s = p.trim_end_matches(seps);
    s.rsplit(seps).next().unwrap_or("").to_string()
}

fn uuid() -> Result<String, Thrown> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| Thrown::new(format!("no random bytes: {e}")))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = xenonet_base::jsstr::hex(&b);
    Ok(format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]))
}

fn obj(fields: Vec<(&str, Arg)>) -> Arg {
    Arg::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The envelope for a message from `sender` ({ id, kind, name, product }),
/// authenticated by `method`, with the route's `verdict` (verify).
pub fn attest(sender: &Arg, method: &Arg, verdict: &Arg, home: &Arg, now: &Arg, platform: &str) -> Result<Arg, Thrown> {
    let m = js_text(method);
    let process = process_of(verdict, method.as_str().unwrap_or(""));
    let strength = strength_of(method.as_str().unwrap_or(""), process);
    let first = match verdict.field("chain") {
        Arg::List(l) => l.first().cloned().unwrap_or(Arg::Undef),
        _ => Arg::Undef,
    };
    let exe = first.field("exe").or(Arg::Null);
    let client = if exe.truthy() {
        let text = js_text(&exe);
        let home = if home.truthy() { js_text(home) } else { String::new() };
        let shown = if home.is_empty() { text.clone() } else { xenonet_base::paths::tilde(&text, &home) };
        obj(vec![("exe", Arg::text(shown)), ("name", Arg::text(basename(&text)))])
    } else {
        Arg::Null
    };
    let transport = match m.as_str() {
        "grant" if method.as_str().is_some() => "tunnel",
        "gateway" if method.as_str().is_some() => "gateway",
        _ => "this machine",
    };
    let mut env = obj(vec![
        ("schema", Arg::text(SCHEMA)),
        ("mid", Arg::text(format!("msg_{}", uuid()?))),
        ("at", now.clone()),
        (
            "sender",
            obj(vec![("id", sender.field("id")), ("kind", sender.field("kind")), ("name", sender.field("name").or(Arg::Null)), ("product", sender.field("product").or(Arg::Null))]),
        ),
        ("auth", obj(vec![("method", method.clone()), ("process", Arg::text(process)), ("strength", Arg::text(strength))])),
        ("client", client),
        ("transport", Arg::text(transport)),
    ]);
    let line = verified_line(&env, platform);
    if let Arg::Obj(f) = &mut env {
        f.push(("line".into(), Arg::text(line)));
    }
    Ok(env)
}

/// How each method proves who sent it, in words.
fn how(method: &Arg) -> String {
    match method.as_str() {
        Some("session") => "its own session's process".into(),
        Some("token") => "its token".into(),
        Some("window") => "XenoNet's own window, with its key".into(),
        Some("grant") => "its remote connection's credential".into(),
        Some("gateway") => "its gateway token alone".into(),
        _ => crate::js::proto_member(&js_text(method)).unwrap_or_else(|| js_text(method)),
    }
}

/// "verified: …": what XenoNet checked, in a sentence, and what it
/// couldn't.
pub fn verified_line(env: &Arg, platform: &str) -> String {
    let sender = env.field("sender");
    let who = if sender.field("kind").as_str() == Some("you") {
        "you, at XenoNet's own window".to_string()
    } else {
        let what = if sender.field("kind").as_str() == Some("actor") { "the actor" } else { "the session" };
        format!("{what} \"{}\" ({})", js_text(&sender.field("name").or(sender.field("id"))), js_text(&sender.field("id")))
    };
    let auth = env.field("auth");
    let method = auth.field("method");
    let how = how(&method);
    let process = match auth.field("process").as_str() {
        Some("signed") => "; XenoNet checked its process and the app's signature".to_string(),
        Some("checked") => {
            let client = env.field("client");
            let name = if client.truthy() { format!(" ({})", js_text(&client.field("name"))) } else { String::new() };
            format!("; XenoNet checked the process it came from{name}")
        }
        Some("unchecked") => {
            let os = match platform {
                "win32" => "Windows",
                "darwin" => "this Mac",
                _ => "Linux",
            };
            let word = if method.as_str() == Some("token") { "the token alone" } else { "its word" };
            format!("; XenoNet can't check which process sent it on {os} yet, so it's {word}")
        }
        _ => String::new(),
    };
    let place = if method.as_str() == Some("gateway") { "from outside this machine, through XenoNet's actor gateway" } else { "on this machine" };
    format!("verified: {who}, by {how}{process}, {place}. This says how XenoNet knows who sent it, not that what it says is true, and it approves nothing.")
}

/// A message's own words as they're shown beneath XenoNet's lines: each of
/// its lines quoted, with what could pass for a line of XenoNet's own
/// taken out (line breaks of other kinds, controls, and characters that
/// reorder text).
pub fn quoted(text: &str) -> String {
    let lines = crate::re!(r"\r\n?|[\u0085\u2028\u2029\u000b\u000c]").replace_all(text, "\n");
    let clean = crate::re!(r"[\u0000-\u0008\u000e-\u001f\u007f-\u0084\u0086-\u009f\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]").replace_all(&lines, "");
    clean.split('\n').map(|l| format!("  > {l}")).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signed_session_is_strong() {
        let verdict = obj(vec![("checked", Arg::Bool(true)), ("sure", Arg::Bool(true)), ("chain", Arg::List(vec![obj(vec![("exe", Arg::text("/Users/me/bin/claude"))])]))]);
        let sender = obj(vec![("id", Arg::text("claude:a")), ("kind", Arg::text("agent")), ("name", Arg::text("Docs"))]);
        let env = attest(&sender, &Arg::text("session"), &verdict, &Arg::text("/Users/me"), &Arg::num(1.0), "darwin").unwrap();
        assert_eq!(env.field("auth").field("strength"), Arg::text("strong"));
        assert_eq!(env.field("client").field("exe"), Arg::text("~/bin/claude"));
        assert!(js_text(&env.field("line")).starts_with("verified: the session \"Docs\" (claude:a), by its own session's process; XenoNet checked its process"));
    }

    #[test]
    fn quoting_takes_out_what_could_pass_for_xenonets_own_lines() {
        assert_eq!(quoted("a\r\nb\u{2028}c\u{202e}d"), "  > a\n  > b\n  > cd");
    }
}
