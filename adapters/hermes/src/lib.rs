//! Hermes Agent's adapter (docs/design/direction.md, "Adapters and
//! surfaces"): its observer, which reads Hermes's state on this machine
//! for metadata only, XenoNet's plugin for Hermes (`plugin`), and the
//! doctor's check on that plugin.

use std::sync::{Arc, OnceLock};
use xenonet_adapter::apps::App;
use xenonet_adapter::observe::{product_app, Observer};
use xenonet_adapter::products::{product_of, Product};
use xenonet_adapter::{Check, Platform};

pub mod checks;
pub mod files;
pub mod observer;
pub mod plugin;

/// The product.
pub fn product() -> &'static Product {
    product_of("hermes-agent").expect("the products table has hermes-agent")
}

/// The adapter.
pub struct Adapter(App);

impl Adapter {
    /// The adapter, as the registry holds one.
    pub fn platform() -> Arc<dyn Platform> {
        static APP: OnceLock<App> = OnceLock::new();
        Arc::new(Adapter(APP.get_or_init(|| product_app(product())).clone()))
    }
}

impl Platform for Adapter {
    fn app(&self) -> &App {
        &self.0
    }

    fn checks(&self) -> Vec<Box<dyn Check>> {
        checks::all()
    }

    fn observer(&self) -> Option<Box<dyn Observer>> {
        Some(Box::new(observer::Hermes::new()))
    }

    // Where its installer links it, ~/.local/bin, for a service whose PATH
    // doesn't have it (one started at login). On Windows it's on the PATH
    // or not found.
    fn program(&self, _env: &xenonet_adapter::dirs::Env, platform: &str, home: &str, look: &mut dyn xenonet_adapter::dirs::Look) -> Option<String> {
        if platform == "win32" {
            return None;
        }
        let at = xenonet_os::paths::join_all(&[home, ".local", "bin", "hermes"]);
        look.exists(&at).then_some(at)
    }
}
