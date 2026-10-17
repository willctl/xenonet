//! XenoNet Dev's instances (core/instances.js): each is its own app, with
//! its own settings, port and single-instance lock, so they run side by
//! side. The command needs their names and ports to say which it drives.

/// An instance: its id, what it's called, and its port.
pub struct Instance {
    pub id: &'static str,
    pub name: &'static str,
    pub port: u16,
}

pub const INSTANCES: &[Instance] = &[
    Instance { id: "dev", name: "XenoNet Dev", port: 47842 },
    Instance { id: "demo", name: "XenoNet Dev (demo)", port: 47841 },
    Instance { id: "test", name: "XenoNet Dev (test)", port: 47843 },
    Instance { id: "film", name: "XenoNet Dev (film)", port: 47844 },
    Instance { id: "installed", name: "XenoNet", port: 47840 },
    Instance { id: "showcase", name: "XenoNet Dev (showcase)", port: 47845 },
];

pub fn instance(id: &str) -> Option<&'static Instance> {
    INSTANCES.iter().find(|i| i.id == id)
}
