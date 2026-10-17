//! core/instances.js: the instances XenoNet Dev runs as from source, each
//! its own app with its own settings, port and single-instance lock, so
//! they run side by side. XENONET_INSTANCE picks one; otherwise
//! XENONET_DEMO=1 means demo, and anything else dev. A frozen instance
//! runs the code as it was at launch.

use xenonet_base::text::js_number;

pub struct Instance {
    pub id: &'static str,
    pub name: &'static str,
    pub port: u16,
    pub demo: bool,
    /// Only the frozen ones say so; the others leave the key out.
    pub frozen: bool,
}

pub const INSTANCES: [Instance; 6] = [
    Instance { id: "dev", name: "XenoNet Dev", port: 47842, demo: false, frozen: false },
    Instance { id: "demo", name: "XenoNet Dev (demo)", port: 47841, demo: true, frozen: false },
    Instance { id: "test", name: "XenoNet Dev (test)", port: 47843, demo: true, frozen: true },
    Instance { id: "film", name: "XenoNet Dev (film)", port: 47844, demo: false, frozen: true },
    Instance { id: "installed", name: "XenoNet", port: 47840, demo: false, frozen: true },
    Instance { id: "showcase", name: "XenoNet Dev (showcase)", port: 47845, demo: true, frozen: true },
];

pub fn instance(id: &str) -> Option<&'static Instance> {
    INSTANCES.iter().find(|i| i.id == id)
}

/// The instance the environment picks, and its port: XENONET_PORT as
/// Number() reads it when it's set, else the instance's own. Err is the
/// message for an id that isn't one.
pub fn instance_from(xenonet_instance: Option<&str>, xenonet_demo: Option<&str>, xenonet_port: Option<&str>) -> Result<(&'static Instance, f64), String> {
    let id = match xenonet_instance {
        Some(id) if !id.is_empty() => id,
        _ if xenonet_demo == Some("1") => "demo",
        _ => "dev",
    };
    let Some(found) = instance(id) else {
        let ids: Vec<&str> = INSTANCES.iter().map(|i| i.id).collect();
        return Err(format!("XENONET_INSTANCE is {id}; it can be {}", ids.join(", ")));
    };
    let port = match xenonet_port {
        Some(p) => js_number(Some(&serde_json::Value::String(p.into()))),
        None => f64::from(found.port),
    };
    Ok((found, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_from_the_environment() {
        assert_eq!(instance_from(None, None, None).unwrap().0.id, "dev");
        assert_eq!(instance_from(Some(""), Some("1"), None).unwrap().0.id, "demo");
        assert_eq!(instance_from(Some("dev"), None, Some("5000")).unwrap().1, 5000.0);
        assert!(instance_from(Some("dev"), None, Some("x")).unwrap().1.is_nan());
        assert_eq!(instance_from(Some("prod"), None, None).err().unwrap(), "XENONET_INSTANCE is prod; it can be dev, demo, test, film, installed, showcase");
    }
}
