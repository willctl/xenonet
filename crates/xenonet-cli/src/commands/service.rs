//! xenonet service (cli/commands/service.js): the background service,
//! started, asked how it is, or stopped, through the component service's
//! socket (core/components/runtime.js, install.js); and started at login
//! under launchd, systemd or Task Scheduler (install, uninstall, run;
//! cores/supervisor.rs, docs/design/always-on.md).

use crate::args::Flags;
use crate::cores;
use crate::cores::supervisor as login;
use crate::out::{c, json as print_json, print, Done, Fail};
use crate::sys;
use serde_json::{json, Map, Value};
use xenonet_base::json::stringify;
use xenonet_components::supervisor::{self, Disk, Files, System, UNSUPPORTED};

pub const HELP: &str = include_str!("help/service.txt");

pub fn run(args: &[String], flags: &Flags) -> Done {
    let action = args.first().map(String::as_str).filter(|a| !a.is_empty()).unwrap_or("status");
    // The environment's XENONET_PORT, or the one --port names.
    let mut env = Map::new();
    if flags.on("port") {
        env.insert("XENONET_PORT".into(), json!(xenonet_base::text::js_string(&flags.get("port").cloned().unwrap_or(Value::Null))));
    }
    let env = Value::Object(env);
    match action {
        "start" => start(&env),
        "status" => Ok(status(&env)),
        "stop" => stop(&env, flags),
        "install" => install(&env, flags),
        "uninstall" => uninstall(&env, flags),
        "run" => login::run(&env, args.get(1..).unwrap_or_default()),
        _ => Err(Fail::error(HELP)),
    }
}

/// The port this command is about.
fn port_of(env: &Value) -> String {
    env.get("XENONET_PORT").and_then(Value::as_str).map(str::to_string).or_else(|| sys::var("XENONET_PORT")).unwrap_or_else(|| "47840".into())
}

/// Stops the service that answers. The stop file goes down first: a
/// supervisor waiting for the port, because a service Desktop started had
/// it before the supervisor's own could start, sees it and stays stopped
/// too, rather than starting its copy the moment this one frees the port.
/// A stop that's refused (terminals open) takes it away again.
fn stop(env: &Value, flags: &Flags) -> Done {
    let file = cores::stop_file(env);
    if let Some(dir) = std::path::Path::new(&file).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&file, stringify(&json!({ "at": crate::out::now(), "pid": std::process::id() }), 0));
    match cores::service_request(env, "stop", &json!({ "force": flags.on("force") })) {
        Ok(answer) => {
            print(stringify(&answer, 0));
            Ok(0)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&file);
            Err(e)
        }
    }
}

/// Starts the service unless one answers. One registered to start at login
/// is started by its supervisor, so there's never a second copy beside
/// the one it keeps. A stop file an earlier stop left is cleared first.
fn start(env: &Value) -> Done {
    let _ = std::fs::remove_file(cores::stop_file(env));
    cores::recover_service_update(env)?;
    if login::running(env).is_some() {
        print(stringify(&json!({ "status": "reused" }), 0));
        return Ok(0);
    }
    if let Some(place) = login::registered_for(&port_of(env)) {
        login::start(env, &place)?;
        print(stringify(&json!({ "status": "started", "supervisor": place.kind.name() }), 0));
        return Ok(0);
    }
    let status = cores::ensure_service(&cores::installed_runtime(env)?)?;
    print(stringify(&json!({ "status": status }), 0));
    Ok(0)
}

/// The running service's own status, with `running`, and `login`: whether
/// it starts at login, and how the supervisor has it. With no service
/// running it says so, and exits 1.
fn status(env: &Value) -> i32 {
    let place = login::place();
    let login = supervisor::status(place.as_ref(), &Disk, &mut System);
    match cores::service_request(env, "status", &json!({})) {
        Ok(mut s) => {
            s["running"] = json!(true);
            s["supervised"] = json!(login::supervised(&s, &login));
            s["login"] = login;
            print(stringify(&s, 0));
            0
        }
        Err(e) => {
            print(stringify(&json!({ "running": false, "error": e.message, "login": login }), 0));
            1
        }
    }
}

fn install(env: &Value, flags: &Flags) -> Done {
    let Some(place) = login::place() else { return Err(Fail::new(UNSUPPORTED, 1)) };
    login::yours(env)?;
    // The launcher the job runs, copied from the Tools when they're there.
    let _ = cores::install_launcher();
    let mut given = cores::env_map();
    for (k, v) in env.as_object().into_iter().flatten() {
        given.insert(k.clone(), v.clone());
    }
    let job = login::job(&given);
    // Reloading a changed registration restarts the service the supervisor
    // runs, and with it its terminals.
    let changed = supervisor::contents(&place, &job).ok() != Disk.read(&place.file);
    if changed {
        if let Some(s) = login::running(env) {
            let seen = supervisor::status(Some(&place), &Disk, &mut System);
            let supervised = login::supervised(&s, &seen);
            if supervised && login::has_terminals(&s) && !flags.on("force") {
                return Err(Fail::new("XenoNet has running terminals, and changing how it starts at login restarts it. Close them first, or use --force.", 1));
            }
            // Task Scheduler's copy is stopped cleanly first, so `service
            // run` ends with it and the new definition starts afresh.
            if supervised && place.kind == supervisor::Kind::TaskScheduler {
                let _ = cores::service_request(env, "stop", &json!({ "force": true }));
            }
        }
    }
    let mut out = supervisor::install(&place, &job, &mut Disk, &mut System).map_err(|e| Fail::new(e, 1))?;
    let takeover = if out["status"] == json!("already") { None } else { login::hand_over(env, &place) };
    out["takeover"] = json!(takeover);
    let linger = supervisor::lingering(&place, &mut System);
    out["linger"] = json!(linger);
    // On a Mac, the tray app starts at login beside it, when there's one
    // installed. The service's registration stands if the tray's fails.
    out["tray"] =
        login::tray_place(&place).map_or(Value::Null, |tp| login::install_tray(&tp, &given, &job).unwrap_or_else(|e| json!({ "status": "failed", "error": e, "app": tp.app })));
    if flags.on("json") {
        print_json(&out);
        return Ok(0);
    }
    let by = place.kind.name();
    if out["status"] == json!("already") {
        print(c::dim(format!("XenoNet's service already starts at login ({by}, {}).", sys::tilde(&place.file))));
        say_tray(&out["tray"]);
        return Ok(0);
    }
    print(format!("{} XenoNet's service starts at login now, and starts again if it stops unexpectedly.", c::green("✓")));
    print(c::dim(format!("  {by} · {}", sys::tilde(&place.file))));
    match takeover {
        Some("now") => print(c::dim(format!("  The service that was running stopped, and the one {by} started took over."))),
        Some("terminals") => print(format!("{} The service running now keeps running, since it has terminals open. The one {by} starts takes over when it stops.", c::yellow("!"))),
        Some("later") => print(c::dim(format!("  The service running now keeps running. The one {by} starts takes over when it stops."))),
        _ => {}
    }
    if linger == Some(false) {
        print(c::dim(format!("  It starts when you log in. To start it at boot, before anyone logs in, run: loginctl enable-linger {}", place.user)));
    }
    say_tray(&out["tray"]);
    Ok(0)
}

/// What became of the tray app, after the service's lines.
fn say_tray(tray: &Value) {
    let app = tray["app"].as_str().map(sys::tilde).unwrap_or_default();
    match tray["status"].as_str() {
        Some("installed" | "updated") => print(format!("{} XenoNet's tray starts at login too, from {app}.", c::green("✓"))),
        Some("already") => print(c::dim(format!("XenoNet's tray already starts at login, from {app}."))),
        Some("removed") => print(format!("{} XenoNet's tray no longer starts at login, and {app} is gone.", c::green("✓"))),
        Some("failed") => print(format!("{} XenoNet's tray, {app}: {}", c::yellow("!"), tray["error"].as_str().unwrap_or("it didn't change"))),
        _ => {}
    }
}

fn uninstall(env: &Value, flags: &Flags) -> Done {
    let Some(place) = login::place() else {
        if flags.on("json") {
            print_json(&json!({ "status": "absent", "supervisor": null, "reason": UNSUPPORTED }));
        } else {
            print(c::dim("XenoNet's service doesn't start at login on this system."));
        }
        return Ok(0);
    };
    login::yours(env)?;
    // Unregistering stops the copy the supervisor runs, terminals and all.
    let mut stops = false;
    if let Some(s) = login::running(env) {
        let seen = supervisor::status(Some(&place), &Disk, &mut System);
        stops = login::supervised(&s, &seen);
        if stops && login::has_terminals(&s) && !flags.on("force") {
            return Err(Fail::new("XenoNet has running terminals, and the service stops when it no longer starts at login. Close them first, or use --force.", 1));
        }
        // Task Scheduler ends the task's own process, which may leave the
        // service it started running: that one is asked to stop first.
        if stops && place.kind == supervisor::Kind::TaskScheduler {
            let _ = cores::service_request(env, "stop", &json!({ "force": true }));
        }
    }
    let mut out = supervisor::uninstall(&place, &mut Disk, &mut System).map_err(|e| Fail::new(e, 1))?;
    out["stopped"] = json!(stops);
    out["tray"] = login::tray_place(&place).map_or(Value::Null, |tp| login::uninstall_tray(&tp).unwrap_or_else(|e| json!({ "status": "failed", "error": e, "app": tp.app })));
    if flags.on("json") {
        print_json(&out);
        return Ok(0);
    }
    if out["status"] == json!("absent") {
        print(c::dim("XenoNet's service wasn't set to start at login."));
        say_tray(&out["tray"]);
        return Ok(0);
    }
    print(format!("{} XenoNet's service no longer starts at login.", c::green("✓")));
    if stops {
        print(c::dim("  It stopped. Opening Desktop, or xenonet service start, starts it until you log out."));
    }
    say_tray(&out["tray"]);
    Ok(0)
}
