//! The container images Adytum knows by name: a registry path and the digest
//! it must have. The digest is the multi-architecture index's, so the same
//! pin serves both CPUs.

use adytum_model::ImagePin;

const PINS: &str = include_str!("../pins.json");

/// The default image name.
pub const DEFAULT_IMAGE: &str = "alpine-3.20";

/// The pin for an image name such as `alpine-3.20`.
pub fn image(name: &str) -> Option<ImagePin> {
    let value = serde_json::from_str::<serde_json::Value>(PINS).ok()?;
    serde_json::from_value(value.get("images")?.get(name)?.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_image_is_pinned_by_digest() {
        let pin = image(DEFAULT_IMAGE).unwrap();
        assert!(pin.location.starts_with("https://"));
        assert!(pin.digest.starts_with("sha256:"));
        assert!(image("nonesuch").is_none());
    }
}
