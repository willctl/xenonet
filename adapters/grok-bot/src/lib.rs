//! Grok Bot's adapter (docs/design/direction.md, "Adapters and
//! surfaces"): its observer, which sees Grok Bot's app on this Mac and
//! the commands it runs.

use std::sync::{Arc, OnceLock};
use xenonet_adapter::apps::App;
use xenonet_adapter::observe::{product_app, Observer};
use xenonet_adapter::products::{product_of, Product};
use xenonet_adapter::Platform;

pub mod observer;

/// The product.
pub fn product() -> &'static Product {
    product_of("grok-bot").expect("the products table has grok-bot")
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

    fn observer(&self) -> Option<Box<dyn Observer>> {
        Some(Box::new(observer::GrokBot::new()))
    }
}
