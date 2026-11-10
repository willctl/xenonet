//! The service's console-only cores: the themes (yours read fresh, and
//! their folder watched), the Spotify remote (the demo's made up), the
//! system panel, and opening things and the xenonet command (`open`),
//! which the composition root puts in place.

pub mod open;
pub mod spotify;
pub mod system;
pub mod themes;

use std::path::Path;
use std::sync::Arc;
use xenonet_actors::secrets::Secrets;
use xenonet_api::cores::Cores;
use xenonet_api::wire::WireEnv;

/// Puts the console's cores in place of the pending ones: the themes from
/// the state folder's themes/, and Spotify, whose settings are the state
/// folder's spotify.json (the demo's player is made up).
pub fn wire(cores: &mut Cores, demo: bool, port: u16, state_dir: &str, env: &WireEnv) {
    cores.themes = Arc::new(themes::ThemeFiles);
    themes::watch(Path::new(state_dir).join("themes").to_string_lossy().into_owned(), cores.store.clone(), env.emitter.clone());
    cores.spotify = if demo {
        Arc::new(spotify::DemoSpotify::new())
    } else {
        let file = Path::new(state_dir).join("spotify.json").to_string_lossy().into_owned();
        Arc::new(spotify::SpotifyRemote::new(&file, port, Arc::new(Secrets::real()), env.emitter.clone()))
    };
}
