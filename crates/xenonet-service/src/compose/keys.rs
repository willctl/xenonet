//! Whose keys an instance gets (docs/design/direction.md, "The machine
//! key"): the installed XenoNet and XenoNet Dev's dev instance use the
//! machine's own (xenonet-keys' `System`: the Secure Enclave and Touch ID
//! on a Mac, polkit or a passphrase on Linux), since they act for the
//! person. The demo, the tests and every other instance get fakes, which
//! never prompt or touch the hardware: file keys in their own state
//! folder, and a presence check that answers yes.

use std::sync::Arc;
use xenonet_id::vault::Vault;

/// The vault for an instance, by its name (None for the installed one)
/// and whether it's the demo.
pub fn vault(instance: Option<&str>, demo: bool) -> Arc<dyn Vault> {
    if !demo && matches!(instance, None | Some("" | "installed" | "dev")) {
        Arc::new(xenonet_keys::System)
    } else {
        Arc::new(xenonet_keys::Fake { allow: true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_installed_and_dev_instances_use_the_machines_keys() {
        // A fake's device key is a file key that doesn't ask by itself.
        let dir = std::env::temp_dir().join(format!("xenonet-compose-keys-{}", std::process::id()));
        for (instance, demo) in [(Some("demo"), false), (Some("test"), false), (Some("net-test"), false), (None, true)] {
            let device = vault(instance, demo).device(&dir).unwrap().unwrap();
            assert!(!device.biometric && !device.signer.asks_person(), "{instance:?}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
