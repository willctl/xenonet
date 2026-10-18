//! xenonet theme (cli/commands/theme.js): the console's themes
//! (core/themes.js, and yours in XenoNet's state folder), which one is in
//! use, and switching or copying one.

use crate::args::Flags;
use crate::js::{list, on, str, t};
use crate::out::{c, color, json, print, table, Done, Fail, Layout};
use crate::reach::need;
use crate::sys;
use serde_json::{json, Map, Value};

pub const HELP: &str = include_str!("help/theme.txt");

/// The themes XenoNet has built in (core/themes.js's BUILT_IN), for the
/// shell's completion.
pub const BUILT_IN: [&str; 8] = ["xenonet", "phosphor", "ochre", "gridline", "helios", "rime", "trench", "eclipse"];

// parseInt(hex.slice(i, i + 2), 16), as text: NaN for what isn't hex.
fn channel(hex: &str, i: usize) -> String {
    let part: String = hex.chars().skip(i).take(2).collect();
    let digits: String = part.chars().take_while(char::is_ascii_hexdigit).collect();
    i64::from_str_radix(&digits, 16).map_or("NaN".into(), |n| n.to_string())
}

/// A swatch of a theme's colors, in terminals that show true color.
fn swatch(colors: &[Value]) -> String {
    if !color() {
        return String::new();
    }
    colors.iter().map(|hex| crate::js::text(Some(hex))).map(|hex| format!("\x1b[48;2;{}m  \x1b[0m", [1, 3, 5].map(|i| channel(&hex, i)).join(";"))).collect::<Vec<_>>().concat()
}

fn only(v: &Value, keys: &[&str]) -> Value {
    let mut m = Map::new();
    for k in keys {
        if let Some(x) = v.get(*k) {
            m.insert((*k).into(), x.clone());
        }
    }
    Value::Object(m)
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let s = need(flags)?;
    let first = args.first().filter(|f| !f.is_empty());
    if first.map(String::as_str) == Some("copy") {
        let (from, id) = (args.get(1).filter(|x| !x.is_empty()), args.get(2).filter(|x| !x.is_empty()));
        let (Some(from), Some(id)) = (from, id) else { return Err(Fail::new("say which theme to copy and what to call yours: xenonet theme copy <theme> <id>", 2)) };
        let out = s.post("/api/themes/copy", &json!({ "from": from, "id": id }), &[])?;
        if flags.on("json") {
            json(&only(&out, &["file"]));
            return Ok(0);
        }
        print(format!("{} {}", c::green("✓"), sys::tilde(&t(&out, "file"))));
        print(c::dim(format!("  Edit it, then xenonet theme {id} to use it; XenoNet follows each save.")));
        return Ok(0);
    }
    if let Some(first) = first {
        let th = s.post("/api/theme", &json!({ "id": first }), &[])?;
        if flags.on("json") {
            json(&only(&th, &["id", "name"]));
            return Ok(0);
        }
        let about = th.get("about").filter(|a| !a.is_null()).map_or(String::new(), |a| crate::js::text(Some(a)));
        print(format!("{} {} {}", c::green("✓"), t(&th, "name"), c::dim(format!("· {about}"))));
        return Ok(0);
    }
    let all = s.get("/api/themes")?;
    if flags.on("json") {
        json(&all);
        return Ok(0);
    }
    let current = all.get("current");
    let rows: Vec<Vec<String>> = list(&all, "themes")
        .iter()
        .map(|th| {
            let whose = if on(th, "builtIn") {
                ""
            } else if on(th, "replacesBuiltIn") {
                " (yours, in place of the built-in)"
            } else {
                " (yours)"
            };
            vec![if th.get("id") == current { c::green("●") } else { " ".into() }, swatch(list(th, "swatch")), t(th, "id"), c::dim(format!("{}{whose}", t(th, "about")))]
        })
        .collect();
    print(table(&rows, Layout::wrapped(0)));
    print(c::dim(format!("\nYours go in {}; xenonet theme copy starts one.", str(&all, "folder").map_or_else(|| t(&all, "folder"), sys::tilde))));
    for p in list(&all, "problems") {
        print(c::yellow(format!("! {}", crate::js::text(Some(p)))));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn channels_of_a_color() {
        assert_eq!([1, 3, 5].map(|i| super::channel("#ff8000", i)), ["255", "128", "0"]);
        assert_eq!(super::channel("#zz", 1), "NaN");
    }
}
