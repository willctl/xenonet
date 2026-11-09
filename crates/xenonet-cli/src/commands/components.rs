//! xenonet components: the components' installer and updater
//! (xenonet-components) for XenoNet Desktop, which runs this rather than an
//! installer of its own. Every answer is JSON on stdout; a failure is its
//! message on stderr and exit 1.

use crate::args::Flags;
use crate::cores;
use crate::js::str;
use crate::out::{print, Done, Fail};
use crate::sys;
use serde_json::{json, Value};
use xenonet_base::json::stringify;

pub const HELP: &str = include_str!("help/components.txt");

const NO_BUNDLE: &str = "Install the XenoNet bundle to add its background service and tools.";

/// How a component stands: "missing", "current" (it has the xenonet
/// binary) or "earlier" (from before 0.4.0, a node and XenoNet's JS).
fn state(component: &str) -> Result<&'static str, Fail> {
    Ok(match cores::current_component(component)? {
        None => "missing",
        Some((_, true)) => "current",
        Some((_, false)) => "earlier",
    })
}

/// The components a bundle carries for this machine.
fn carried(bundle: &str) -> Result<Vec<String>, Fail> {
    let file = sys::join(bundle, "components.json");
    let text = std::fs::read_to_string(&file).map_err(|e| Fail::error(xenonet_base::fs::FsError::of(e, "open", &file).message))?;
    let manifest = xenonet_base::json::parse(&text).map_err(|e| Fail::error(e.to_string()))?;
    let m = cores::machine();
    Ok(manifest
        .get("packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|p| str(p, "platform") == Some(m.platform.as_str()) && str(p, "arch") == Some(m.arch.as_str()))
        .filter_map(|p| str(p, "component").map(str::to_string))
        .collect())
}

/// What Desktop does as it starts: the Service and Tools in place. A
/// bundle (the Mac's disk image, Windows' XenoNet Setup) brings its own for
/// any that's missing or from before 0.4.0; any it doesn't bring comes from
/// the latest release, Tools first, so the service names their launcher.
/// One from before 0.4.0 that can't be replaced now (a service with a
/// terminal open, or an update that failed and was rolled back) runs on
/// until Desktop starts again. Only a missing component, or a failure with
/// nothing from before 0.4.0 to explain it, is an error.
fn ready(bundle: Option<&str>, test: bool) -> Result<Value, Fail> {
    let mut problems: Vec<String> = vec![];
    let mut waiting = false;
    let mut brought = vec![];
    if let Some(bundle) = bundle {
        brought = carried(bundle)?;
        match cores::install_bundle(bundle, !test) {
            Ok(_) => waiting = brought.iter().any(|c| c == "service") && state("service")? == "earlier",
            Err(e) => problems.push(e.message),
        }
    }
    let mut wanted = vec![];
    for c in ["tools", "service"] {
        let s = state(c)?;
        if s == "missing" || (s == "earlier" && !brought.iter().any(|b| b == c)) {
            wanted.push(c);
        }
    }
    if !wanted.is_empty() && test {
        return Err(Fail::error("The test bundle is missing its components."));
    }
    let mut available: Option<Vec<Value>> = None;
    for c in wanted {
        let result = (|| -> Result<(), Fail> {
            if available.is_none() {
                available = Some(cores::component_updates()?);
            }
            let item = available.iter().flatten().find(|p| str(p, "component") == Some(c)).cloned().ok_or_else(|| Fail::error(NO_BUNDLE))?;
            if str(&cores::update_component(&item)?, "status") == Some("staged") {
                waiting = true;
            }
            Ok(())
        })();
        if let Err(e) = result {
            problems.push(e.message);
        }
    }
    let mut earlier = vec![];
    let mut missing = false;
    for c in ["service", "tools"] {
        match state(c)? {
            "earlier" => earlier.push(c),
            "missing" => missing = true,
            _ => {}
        }
    }
    if missing || (!problems.is_empty() && earlier.is_empty()) {
        return Err(Fail::error(problems.first().cloned().unwrap_or_else(|| NO_BUNDLE.into())));
    }
    Ok(json!({ "earlier": earlier, "waiting": waiting, "problems": problems }))
}

fn arg(args: &[String], i: usize) -> Result<&str, Fail> {
    args.get(i).map(String::as_str).filter(|a| !a.is_empty()).ok_or_else(|| Fail::error(HELP))
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let answer = match args.first().map(String::as_str) {
        Some("ready") => ready(args.get(1).map(String::as_str).filter(|b| !b.is_empty()), flags.on("test"))?,
        Some("bundle") => cores::install_bundle(arg(args, 1)?, !flags.is_false("command"))?,
        Some("runtime") => {
            cores::recover_service_update(&json!({}))?;
            cores::installed_runtime(&json!({}))?
        }
        Some("updates") => Value::Array(cores::component_updates()?),
        Some("update") => {
            let component = arg(args, 1)?;
            if !["service", "tools"].contains(&component) {
                return Err(Fail::error(HELP));
            }
            let item =
                cores::component_updates()?.into_iter().find(|p| str(p, "component") == Some(component)).ok_or_else(|| Fail::error("This component has no published update."))?;
            cores::update_component(&item)?
        }
        Some("activate") => {
            let item = xenonet_base::json::parse(arg(args, 2)?).map_err(|e| Fail::error(format!("<item> isn't JSON: {e}")))?;
            cores::activate_package(arg(args, 1)?, &item)?
        }
        Some("current") => match cores::current_component(arg(args, 1)?)? {
            Some((component, binary)) => json!({ "component": component, "binary": binary }),
            None => Value::Null,
        },
        Some("launcher") => cores::install_launcher()?.unwrap_or(Value::Null),
        _ => return Err(Fail::error(HELP)),
    };
    print(stringify(&answer, 0));
    Ok(0)
}
