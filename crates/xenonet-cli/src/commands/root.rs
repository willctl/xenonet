//! xenonet root (cli/commands/root.js): the projects folder XenoNet reads,
//! where that comes from, and reading another one (core/workspace.js,
//! through a running XenoNet's API).

use crate::args::Flags;
use crate::js::{list, num, on, str, t};
use crate::out::{ago, c, json, now, plural, print, Done, Fail};
use crate::reach::need;
use crate::sys;
use serde_json::{json, Value};
use xenonet_base::text::truthy;

pub const HELP: &str = include_str!("help/root.txt");

fn from_words(source: &str) -> String {
    match source {
        "env" => "set by XENONET_PROJECTS_DIR".into(),
        "chosen" => "chosen in XenoNet".into(),
        "default" => "the default".into(),
        s => s.into(),
    }
}

fn found(sc: &Value) -> String {
    format!("{} in {}", plural(num(sc, "projects"), "project"), plural(num(sc, "groups"), "group"))
}

/// A folder as given here. ~ is left for XenoNet, which reads it as its own
/// home folder (the demo has one of its own); anything else is taken from
/// where the command runs.
pub fn folder(arg: &str) -> String {
    let rest = arg.strip_prefix('~');
    if rest.is_some_and(|r| r.is_empty() || r.starts_with('/') || r.starts_with('\\')) {
        arg.to_string()
    } else {
        sys::resolve(arg)
    }
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let dir = args.first().filter(|d| !d.is_empty());
    if flags.on("reset") && dir.is_some() {
        return Err(Fail::new("give a folder, or --reset for the default, not both", 2));
    }
    if flags.on("check") && dir.is_none() {
        return Err(Fail::new("say which folder to check: xenonet root --check <folder>", 2));
    }
    let s = need(flags)?;
    let info = s.get("/api/workspace/root")?;
    let home = str(&info, "home").unwrap_or("").to_string();
    let shown = |v: &Value, k: &str| match v.get(k) {
        Some(Value::String(p)) => xenonet_base::paths::tilde(p, &home),
        other => crate::js::text(other),
    };

    if flags.on("check") {
        let r = s.post("/api/workspace/check", &json!({ "path": folder(dir.unwrap()) }), &[])?;
        if flags.on("json") {
            json(&r);
        } else if !on(&r, "ok") {
            print(format!("{} {}", c::red("✗"), t(&r, "error")));
        } else {
            let what = match r.get("scan").filter(|sc| truthy(Some(sc))) {
                Some(sc) => format!("{} {}", found(sc), c::dim(format!("({} read in {} ms)", plural(num(sc, "dirsRead"), "folder"), t(sc, "ms")))),
                None => "still scanning".into(),
            };
            print(format!("{} {}: {what}{}", c::green("✓"), shown(&r, "path"), if on(&r, "current") { c::dim(" · the folder XenoNet reads now") } else { String::new() }));
            for w in list(&r, "warnings") {
                print(c::yellow(format!("! {}", crate::js::text(Some(w)))));
            }
        }
        return Ok(if on(&r, "ok") { 0 } else { 1 });
    }

    if dir.is_some() || flags.on("reset") {
        let r = if flags.on("reset") { s.del("/api/workspace/root")? } else { s.post("/api/workspace/root", &json!({ "path": folder(dir.unwrap()) }), &[])? };
        if flags.on("json") {
            json(&r);
            return Ok(0);
        }
        let scan = r.get("scan").filter(|sc| on(sc, "ok")).map_or(String::new(), |sc| format!(": {}", found(sc)));
        print(if on(&r, "changed") {
            format!("{} XenoNet reads {} now{scan}", c::green("✓"), shown(&r, "path"))
        } else {
            format!("{} XenoNet reads {} already{scan}", c::dim("·"), shown(&r, "path"))
        });
        if on(&r, "changed") {
            print(c::dim(format!("  The layout, your rules and Codex's picks for {} stay, for when you come back.", shown(&info, "path"))));
        }
        return Ok(0);
    }

    if flags.on("json") {
        json(&info);
        return Ok(0);
    }
    print(format!("{} {} {}", c::green("●"), c::bold(shown(&info, "path")), c::dim(from_words(&t(&info, "source")))));
    let scanned = match info.get("scan").filter(|sc| !sc.is_null()) {
        Some(sc) if on(sc, "ok") => {
            let cut = if on(sc, "truncated") { c::yellow(format!(", stopped at the {}-folder limit", t(sc, "maxDirs"))) } else { String::new() };
            format!("{}{cut} {}", found(sc), c::dim(format!("(scanned {})", ago(num(sc, "at"), now()))))
        }
        Some(sc) if truthy(Some(sc)) => c::red("XenoNet can't read it"),
        _ => c::dim("not scanned yet"),
    };
    print(format!("  {scanned}"));
    match str(&info, "source") {
        Some("env") => print(c::dim("  To choose a folder with xenonet root or in the console, unset XENONET_PROJECTS_DIR and restart XenoNet.")),
        Some("chosen") => print(c::dim(format!("  xenonet root --reset goes back to {}.", shown(&info, "default")))),
        _ => print(c::dim("  xenonet root <folder> reads another.")),
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn tilde_is_left_for_xenonet() {
        assert_eq!(super::folder("~/Clients"), "~/Clients");
        assert_eq!(super::folder("~"), "~");
        assert!(super::folder("~x").ends_with("/~x") || cfg!(windows));
    }
}
