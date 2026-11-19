//! Key names to BiDi key actions.

use serde_json::{json, Value};

use crate::error::{Error, Result};

fn named(name: &str) -> Option<char> {
    let code = match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => '\u{E007}',
        "tab" => '\u{E004}',
        "escape" | "esc" => '\u{E00C}',
        "backspace" => '\u{E003}',
        "delete" => '\u{E017}',
        "space" => ' ',
        "arrowup" | "up" => '\u{E013}',
        "arrowdown" | "down" => '\u{E015}',
        "arrowleft" | "left" => '\u{E012}',
        "arrowright" | "right" => '\u{E014}',
        "home" => '\u{E011}',
        "end" => '\u{E010}',
        "pageup" => '\u{E00E}',
        "pagedown" => '\u{E00F}',
        "control" | "ctrl" => '\u{E009}',
        "shift" => '\u{E008}',
        "alt" => '\u{E00A}',
        "meta" | "cmd" | "command" => '\u{E03D}',
        other => {
            let n: u32 = other.strip_prefix('f')?.parse().ok().filter(|n| (1..=12).contains(n))?;
            char::from_u32(0xE031 + n - 1)?
        }
    };
    Some(code)
}

fn is_modifier(key: char) -> bool {
    matches!(key, '\u{E009}' | '\u{E008}' | '\u{E00A}' | '\u{E03D}')
}

fn key_of(part: &str, whole: &str) -> Result<char> {
    if let Some(c) = named(part) {
        return Ok(c);
    }
    let mut chars = part.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(c),
        _ => Err(Error::BadArguments(format!(
            "{whole:?} is not a key I know. Use a name such as Enter, Tab, Escape, ArrowDown, F5, a single character, or a combination such as Control+a."
        ))),
    }
}

/// The key actions for `key`: a name (`Enter`), a character (`a`) or a
/// combination (`Control+Shift+a`). Modifiers go down first and come up in
/// reverse order.
pub fn press_actions(key: &str) -> Result<Value> {
    let parts: Vec<&str> = if key == "+" { vec!["+"] } else { key.split('+').collect() };
    let codes = parts.iter().map(|p| key_of(p, key)).collect::<Result<Vec<char>>>()?;
    let (last, mods) = codes.split_last().ok_or_else(|| Error::BadArguments("Give a key to press.".to_owned()))?;
    if let Some(bad) = mods.iter().find(|m| !is_modifier(**m)) {
        return Err(Error::BadArguments(format!("{key:?} has {bad:?} before the last key, which is not Control, Shift, Alt or Meta.")));
    }
    let mut actions = Vec::new();
    for m in mods {
        actions.push(json!({ "type": "keyDown", "value": m.to_string() }));
    }
    actions.push(json!({ "type": "keyDown", "value": last.to_string() }));
    actions.push(json!({ "type": "keyUp", "value": last.to_string() }));
    for m in mods.iter().rev() {
        actions.push(json!({ "type": "keyUp", "value": m.to_string() }));
    }
    Ok(json!([{ "type": "key", "id": "keyboard", "actions": actions }]))
}
