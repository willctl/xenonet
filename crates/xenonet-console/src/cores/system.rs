//! The console's system panel: what the computer running XenoNet is doing
//! (xenonet-os's system.rs reads it), and the demo's computer, made up.

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use xenonet_api::cores::{SystemStats, R};
use xenonet_os::stats;
use xenonet_os::system::System;
use xenonet_text::time::now_ms;

/// This computer, read while a window looks.
pub struct SystemPanel(pub Arc<System>);

#[async_trait]
impl SystemStats for SystemPanel {
    async fn read(&self) -> R<Value> {
        Ok(self.0.read().await)
    }

    fn stop(&self) {
        self.0.stop();
    }
}

/// The demo's computer: it breathes slowly, the same way each time the
/// clock comes round.
pub struct DemoSystem;

#[async_trait]
impl SystemStats for DemoSystem {
    async fn read(&self) -> R<Value> {
        Ok(stats::demo_reading(now_ms()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn this_computer_as_the_panel_reads_it() {
        let p = SystemPanel(System::new());
        let r = p.read().await.unwrap();
        assert!(r["cpu"]["threads"].as_u64().unwrap() >= 1);
        assert_eq!(p.read().await.unwrap()["at"], r["at"], "a fresh reading is the last one");
        p.stop();
        assert!(DemoSystem.read().await.unwrap()["cpu"]["cores"].as_array().unwrap().len() > 1);
    }
}
