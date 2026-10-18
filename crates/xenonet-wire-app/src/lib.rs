//! The binary's drivers for the app's parts (themes, open, command,
//! Spotify, the decision model, system stats…), hosts, the assistant,
//! components, integrations and the doctor: their tasks' fetches, children
//! and timers, natively, where the twins' JS answers the same Rust during
//! the port.
//!
//! Each module drives the cores of the JS modules it names, as
//! server/index.js makes them: the demo's are made up or kept in memory,
//! and never reach your machine's own credential store or apps.

// A core's error carries what the JS errors do (a status, a code, extra
// fields), as the routes answer it, and is rare: boxing it would only make
// each driver wordier, as in the service itself.
#![expect(clippy::result_large_err, reason = "a core's error is the routes' own, and rare")]

pub mod assistant;
pub mod decide;
pub mod doctor;
pub mod fx;
pub mod hosts;
pub mod integrations;
pub mod mcp;
pub mod open;
pub mod paths;
pub mod refusals;
pub mod secrets;
pub mod spotify;
pub mod system;
pub mod themes;

use secrets::Secrets;
use std::path::Path;
use std::sync::Arc;
use xenonet_service::config::Config;
use xenonet_service::cores::Cores;
use xenonet_service::wire::WireEnv;

/// Puts this area's cores in place of the pending ones.
pub fn wire(cores: &mut Cores, config: &mut Config, env: &WireEnv) {
    // The demo's world moves every folder, so it's made before anything
    // here takes one: the map's driver may have made it already; else it's
    // made now, as the service would, and the service keeps it.
    if config.demo && config.world.is_none() {
        match cores.demo.create_world() {
            Ok(world) => config.use_world(&world),
            Err(_) => config.demo_folders(),
        }
    }
    let demo = config.demo;
    let state_dir = config.state_dir.clone();
    let js = paths::js_config(config);

    cores.themes = Arc::new(themes::ThemeFiles);
    themes::watch(Path::new(&state_dir).join("themes").to_string_lossy().into_owned(), env.emitter.clone());
    cores.opener = Arc::new(open::Open);
    cores.command = Arc::new(open::XenonetCommand::new());
    cores.refusals = Arc::new(refusals::RefusalStore::new(&state_dir, &config.home));

    // Decisions and OpenRouter's spend share the key, in your credential
    // store; the demo's in memory.
    let decision_secrets = Arc::new(if demo { Secrets::memory() } else { Secrets::real() });
    let decisions = Arc::new(decide::Decisions::new(decision_secrets.clone(), env.emitter.clone()));
    cores.decider = decisions.clone();
    cores.openrouter = if demo { Arc::new(decide::DemoSpent) } else { Arc::new(decide::Spent::new(decision_secrets, env.emitter.clone())) };

    cores.spotify = if demo {
        Arc::new(spotify::DemoSpotify::new())
    } else {
        let file = Path::new(&state_dir).join("spotify.json").to_string_lossy().into_owned();
        Arc::new(spotify::SpotifyRemote::new(&file, config.port, Arc::new(Secrets::real()), env.emitter.clone()))
    };
    cores.system = if demo { Arc::new(system::DemoSystem) } else { Arc::new(system::SystemPanel(system::System::new())) };
    cores.mcp_info = Arc::new(mcp::Catalog::new(&js, &env.version));

    // Hosts: this XenoNet as one, which needs no flag, and with
    // XENONET_HOSTS=1 the hub, whose hosts' file is one of the map's
    // building blocks.
    let schemas = Arc::new(hosts::schemas::HostSchemas::new());
    let host_store = Arc::new(hosts::config::HostsStore::new(&state_dir, schemas.clone()));
    cores.hosts = Arc::new(hosts::config::Hosts(host_store.clone()));
    if cores.hub.is_some() {
        let windows = serde_json::json!({ "activeWindowMs": js["activeWindowMs"], "recentWindowMs": js["recentWindowMs"], "projectActiveMs": js["projectActiveMs"] });
        let exec = fx::env_set("XENONET_EXEC");
        let parts = hosts::hub::HubParts {
            hosts: host_store.clone(),
            schemas,
            state: cores.state.clone(),
            store: cores.store.clone(),
            cache_dir: config.cache_dir.clone(),
            root: config.root.to_string_lossy().into_owned(),
            home: config.home.clone(),
            demo,
            windows,
            emitter: env.emitter.clone(),
            run_as_node: exec.is_some(),
            exec: exec.or_else(|| fx::on_path("node")).unwrap_or_else(|| "node".into()),
            ssh_bin: vec!["ssh".into()],
        };
        cores.hub = Some(Arc::new(hosts::hub::HubCore(hosts::hub::HostHub::new(parts))));
        let block = host_store.clone();
        let _ = cores.state.add_block(Arc::new(move || block.health()), None);
    }

    // Remote actors: their connections, reads, audit and tunnels, over
    // the engine's own store, with actors and the assistant both on. The
    // reads' source needs no store, so it's real either way.
    if cores.connections.is_some() {
        let reads_of =
            |remote, store| assistant::reads::Reads::new(remote, cores.state.clone(), cores.actors.clone(), cores.engine_store.clone(), store, &env.version, env.emitter.clone());
        let native = cores.engine_store.native().and_then(|a| a.downcast_ref::<xenonet_engine::store::Shared>().cloned());
        cores.assistant_source = Some(Arc::new(assistant::reads::SourceCore(reads_of(None, None))));
        if let (Some(store), Some(actors)) = (native, cores.actors.clone()) {
            let dir = xenonet_assistant::credentials::assistant_dir(&state_dir);
            match assistant::connections::Remote::start(store.clone(), &dir, &state_dir, config.port, actors, cores.engine_store.clone(), cores.state.clone()) {
                Ok(remote) => {
                    let reads = reads_of(Some(remote.clone()), Some(store.clone()));
                    cores.connections = Some(Arc::new(assistant::connections::ConnectionsCore(remote.clone())));
                    cores.assistant_source = Some(Arc::new(assistant::reads::SourceCore(reads.clone())));
                    cores.assistant = Some(Arc::new(assistant::reads::ServiceCore(reads.clone())));
                    cores.audit = Some(Arc::new(assistant::reads::AuditCore(reads)));
                    // tunnel-client runs XenoNet's launcher as the adapter, as
                    // the hooks do.
                    let (launcher, exec) = (fx::env_set("XENONET_LAUNCHER_EXEC"), fx::env_set("XENONET_EXEC"));
                    let launch = assistant::tunnels::Launch {
                        run_as_node: launcher.is_none() && exec.is_some(),
                        exec: launcher.or(exec).or_else(|| fx::on_path("node")).unwrap_or_else(|| "node".into()),
                        entry: config.cli_entry().to_string_lossy().into_owned(),
                        port: config.port,
                    };
                    let secrets = Arc::new(if demo { Secrets::memory() } else { Secrets::real() });
                    let runner = assistant::tunnels::Runner::new(remote, store, &dir, secrets, launch, &config.home, demo, env.emitter.clone());
                    cores.tunnels = Some(Arc::new(assistant::tunnels::TunnelsCore(runner)));
                }
                Err(e) => env.emitter.log("error", "assistant.start", &format!("remote actors' connections couldn't start: {}", e.message)),
            }
        }
    }

    // Integrations: their records in the state folder, the demo's in memory.
    let parts = integrations::Parts {
        dir: (!demo).then(|| Path::new(&state_dir).join("integrations").to_string_lossy().into_owned()),
        home: config.home.clone(),
        demo,
        decider: decisions.clone(),
        state: cores.state.clone(),
        root: config.root.to_string_lossy().into_owned(),
    };
    match integrations::Integrated::new(parts) {
        Ok(i) => cores.integrations = Arc::new(integrations::IntegrationsCore(i)),
        Err(e) => env.emitter.log("error", "integrations.start", &format!("the integrations' records can't be kept: {}", e.message)),
    }

    // The doctor last: its context is the other cores (the hub's hosts
    // among them), as server/index.js's doctorContext reads them.
    let parts =
        Arc::new(doctor::Parts { cores: cores.clone(), config: config.clone(), paths: js, version: env.version.clone(), drift: env.drift.clone(), emitter: env.emitter.clone() });
    let service = doctor::DoctorService::new(parts.clone());
    cores.doctor = Arc::new(doctor::DoctorCore(service.clone()));
    cores.diagnostics = Arc::new(doctor::Diagnose { parts, doctor: service });
}
