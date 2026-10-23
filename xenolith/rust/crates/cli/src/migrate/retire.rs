//! Windows: agent-wiki's service (AgentWiki, run as NT SERVICE\AgentWiki) goes in the same elevated
//! step that registers Xenolith's, so moving costs one UAC prompt. `xenolith install` takes
//! --retire-agent-wiki-service (with each folder whose grant goes too as --retire-grant <folder>)
//! and hands it on to the elevated install-service, which stops and deletes the service and removes
//! the account's folder grants before it registers Xenolith's. Stopping the service needs no
//! administrator rights (agent-wiki's installer gave you that right); deleting it does.

use std::path::PathBuf;

pub const FLAG: &str = "--retire-agent-wiki-service";
pub const GRANT: &str = "--retire-grant";

/// The folders whose grants go with the service, when the flag was given.
pub fn requested(args: &[String]) -> Option<Vec<PathBuf>> {
    if !args.iter().any(|a| a == FLAG) {
        return None;
    }
    Some(args.windows(2).filter(|w| w[0] == GRANT).map(|w| PathBuf::from(&w[1])).collect())
}

/// The arguments that carry the request (to install, and on to the elevated install-service).
pub fn forward(grants: &[PathBuf]) -> Vec<String> {
    let mut out = vec![FLAG.to_string()];
    for g in grants {
        out.extend([GRANT.to_string(), g.to_string_lossy().into_owned()]);
    }
    out
}

/// Whether the AgentWiki service is registered (no elevation needed to ask).
#[cfg(windows)]
pub fn service_exists() -> bool {
    crate::install::windows::sc(&["query", super::detect::AW_SERVICE]).ok
}

/// Stops and deletes the AgentWiki service and removes its account's grants: one line per action,
/// for the elevated step's log. Runs elevated.
#[cfg(windows)]
pub fn retire(grants: &[PathBuf]) -> Vec<String> {
    use crate::install::places::fwd;
    use crate::install::windows::sc;
    use std::time::{Duration, Instant};
    let name = super::detect::AW_SERVICE;
    let mut log = vec![];
    if sc(&["query", name]).ok {
        let _ = sc(&["stop", name]);
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && !sc(&["query", name]).stdout.contains("STOPPED") {
            std::thread::sleep(Duration::from_millis(300));
        }
        let d = sc(&["delete", name]);
        log.push(if d.ok { format!("agent-wiki's {name} service deleted") } else { format!("! sc delete {name}: {}", d.last_line()) });
    }
    let system32 = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join("System32");
    for g in grants.iter().filter(|g| g.exists()) {
        let r = crate::install::sys::run(&system32.join("icacls.exe"), &[&g.to_string_lossy(), "/remove:g", &format!("NT SERVICE\\{name}"), "/Q"]);
        log.push(if r.ok { format!("removed NT SERVICE\\{name} from {}", fwd(g)) } else { format!("! icacls {}: {}", fwd(g), r.last_line()) });
    }
    log
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_round_trips() {
        let grants = vec![PathBuf::from("C:/Users/x/My Wiki"), PathBuf::from("C:/Users/x/AppData/Local/AgentWiki")];
        let mut args = vec!["--wiki-dir".to_string(), "C:/w".into()];
        assert_eq!(requested(&args), None);
        args.extend(forward(&grants));
        assert_eq!(requested(&args), Some(grants));
    }
}
