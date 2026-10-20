//! xenonet update (cli/commands/update.js): service and tools updates,
//! checked or installed (core/components/updates.js).

use crate::args::Flags;
use crate::cores;
use crate::js::str;
use crate::out::{print, Done, Fail};
use xenonet_base::json::stringify;

pub const HELP: &str = include_str!("help/update.txt");

pub fn run(args: &[String], flags: &Flags) -> Done {
    let component = args.first().map(String::as_str).filter(|a| !a.is_empty()).unwrap_or("all");
    if !["service", "tools", "all"].contains(&component) {
        return Err(Fail::error(HELP));
    }
    let updates = cores::component_updates()?;
    for item in updates.iter().filter(|i| component == "all" || str(i, "component") == Some(component)) {
        let out = if flags.on("check") { item.clone() } else { cores::update_component(item)? };
        print(stringify(&out, 0));
    }
    Ok(0)
}
