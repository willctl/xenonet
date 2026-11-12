//! The app's cores, made and put in place: opening things and the
//! xenonet command, refusals, the decision model and OpenRouter's spend,
//! the system panel, the MCP catalog, remote actors' connections, reads
//! and tunnels, the integrations and the doctor. Each
//! is its domain crate's; the demo's are made up or kept in memory, and
//! never reach your machine's own credential store or apps.

mod paths;

use super::assistant;
use super::integrations::{self, decide};
use super::open;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use xenonet_actors::secrets::Secrets;
use xenonet_api::cores::Cores;
use xenonet_api::wire::WireEnv;
use xenonet_components::runtime::xenonet_bin_here;
use xenonet_console::cores::system;
use xenonet_doctor::runtime as doctor;
use xenonet_mcp_catalog::catalog as mcp;
use xenonet_sources::refusals::runtime as refusals;

/// What these cores run with: the service's settings it reads, once the
/// demo's world has moved their folders.
#[derive(Clone, Debug)]
pub struct Settings {
    pub demo: bool,
    pub dev: bool,
    pub port: u16,
    /// The app's name, which the desktop app passes on.
    pub name: String,
    /// installed, demo, dev, or the instance XenoNet Dev names.
    pub instance: Option<String>,
    /// XENONET_LAUNCHER_EXEC: installed XenoNet's launcher started it.
    pub launcher_exec: Option<String>,
    pub home: String,
    pub projects_dir: String,
    pub state_dir: String,
    pub cache_dir: String,
    /// Where XenoNet's files are.
    pub root: PathBuf,
    /// The demo's world, when it was made.
    pub world: Option<Value>,
}

/// Puts this area's cores in place of the pending ones.
pub fn wire(cores: &mut Cores, config: &Settings, env: &WireEnv) {
    let demo = config.demo;
    let state_dir = config.state_dir.clone();
    let js = paths::config_json(config);

    cores.opener = Arc::new(open::Open);
    cores.command = Arc::new(open::XenonetCommand::new());
    cores.refusals = Arc::new(refusals::RefusalStore::new(&state_dir, &config.home));

    // Decisions and OpenRouter's spend share the key, in your credential
    // store; the demo's in memory.
    let decision_secrets = Arc::new(if demo { Secrets::memory() } else { Secrets::real() });
    let decisions = Arc::new(decide::Decisions::new(decision_secrets.clone(), env.emitter.clone()));
    cores.decider = decisions.clone();
    cores.openrouter = if demo { Arc::new(decide::DemoSpent) } else { Arc::new(decide::Spent::new(decision_secrets, env.emitter.clone())) };

    cores.system = if demo { Arc::new(system::DemoSystem) } else { Arc::new(system::SystemPanel(xenonet_os::system::System::new())) };
    cores.mcp_info = Arc::new(mcp::Catalog::new(&js, &env.version, cores.state.clone(), xenonet_grok_build::connect::toml::read_settings));

    // Remote actors: their connections, reads, audit and tunnels, over
    // the engine's own store, with actors and the assistant both on. The
    // reads' source needs no store, so it's real either way.
    if cores.connections.is_some() {
        let reads_of =
            |remote, store| assistant::reads::Reads::new(remote, cores.state.clone(), cores.actors.clone(), cores.engine_store.clone(), store, &env.version, env.emitter.clone());
        let native = cores.engine_store.native().and_then(|a| a.downcast_ref::<xenonet_store::engine::Shared>().cloned());
        cores.assistant_source = Some(Arc::new(assistant::reads::SourceCore(reads_of(None, None))));
        if let (Some(store), Some(actors)) = (native, cores.actors.clone()) {
            let dir = xenonet_actors::assistant::credentials::assistant_dir(&state_dir);
            match assistant::connections::Remote::start(store.clone(), &dir, &state_dir, config.port, actors, cores.engine_store.clone(), cores.state.clone(), cores.grants.clone())
            {
                Ok(remote) => {
                    let reads = reads_of(Some(remote.clone()), Some(store.clone()));
                    cores.connections = Some(Arc::new(assistant::connections::ConnectionsCore(remote.clone())));
                    cores.assistant_source = Some(Arc::new(assistant::reads::SourceCore(reads.clone())));
                    cores.assistant = Some(Arc::new(assistant::reads::ServiceCore(reads.clone())));
                    cores.audit = Some(Arc::new(assistant::reads::AuditCore(reads)));
                    // tunnel-client runs the xenonet binary as the adapter, as
                    // the hooks do.
                    let launch = assistant::tunnels::Launch { bin: xenonet_bin_here(&config.home), port: config.port };
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
        decider: decisions,
        state: cores.state.clone(),
        root: config.root.to_string_lossy().into_owned(),
    };
    match integrations::Integrated::new(parts) {
        Ok(i) => cores.integrations = Arc::new(integrations::IntegrationsCore(i)),
        Err(e) => env.emitter.log("error", "integrations.start", &format!("the integrations' records can't be kept: {}", e.message)),
    }

    // The actors' forwards over this machine's endpoint, decided on the
    // engine's grants.
    super::forward::wire(cores);

    // The doctor last: its context is the other cores.
    let settings = doctor::Settings {
        demo,
        port: config.port,
        name: config.name.clone(),
        instance: config.instance.clone(),
        launcher_exec: config.launcher_exec.clone(),
        home: config.home.clone(),
        state_dir: state_dir.clone(),
        root: config.root.clone(),
    };
    let parts = Arc::new(doctor::Parts {
        cores: cores.clone(),
        config: settings,
        paths: js,
        version: env.version.clone(),
        drift: env.drift.clone(),
        emitter: env.emitter.clone(),
        gateway: env.gateway.clone(),
        standing: store_standing,
        machine_key: super::keys::machine_standing(config.instance.as_deref(), demo),
        net: crate::serve::kept_network(cores),
    });
    let service = doctor::DoctorService::new(parts.clone());
    cores.doctor = Arc::new(doctor::DoctorCore(service.clone()));
    cores.diagnostics = Arc::new(doctor::Diagnose { parts, doctor: service });
}

/// The credential store the console's Spotify remote keeps its token in:
/// your computer's, or memory for the demo.
pub struct Vault(Secrets);

/// The vault for the console's cores (`xenonet_console::wire`).
pub fn vault(demo: bool) -> Arc<dyn xenonet_console::cores::spotify::Vault> {
    Arc::new(Vault(if demo { Secrets::memory() } else { Secrets::real() }))
}

#[async_trait::async_trait]
impl xenonet_console::cores::spotify::Vault for Vault {
    async fn has(&self, target: &str) -> Result<bool, String> {
        self.0.has(target).await
    }
    async fn get(&self, target: &str) -> Result<Option<String>, String> {
        self.0.get(target).await
    }
    async fn set(&self, target: &str, secret: &str, comment: &str) -> Result<bool, String> {
        self.0.set(target, secret, comment).await
    }
    async fn remove(&self, target: &str) -> Result<bool, String> {
        self.0.remove(target).await
    }
}

/// Why the store in a state folder can't be used, if it can't: data from
/// an earlier build, or another version's store, which the machine is set
/// up again for.
fn store_standing(state_dir: &str) -> Option<String> {
    match xenonet_store::standing(Path::new(state_dir)) {
        xenonet_store::Standing::SetUpAgain(why) => Some(why),
        xenonet_store::Standing::None | xenonet_store::Standing::Ready => None,
    }
}
