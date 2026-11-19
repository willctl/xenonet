//! The images Adytum knows by name. Each is a location and the hash the
//! download must have, so a changed or swapped image is refused by Lima.

use adytum_model::ImagePin;

const PINS: &str = include_str!("../pins.json");

/// The default image name.
pub const DEFAULT_IMAGE: &str = "ubuntu-24.04";

fn all() -> Vec<(String, ImagePin)> {
    // pins.json is part of this crate and checked by a test, so a bad file
    // is a build problem rather than something a person can cause.
    let Ok(value) = serde_json::from_str::<serde_json::Value>(PINS) else { return Vec::new() };
    let Some(images) = value.get("images").and_then(serde_json::Value::as_object) else { return Vec::new() };
    images.iter().filter_map(|(name, pin)| serde_json::from_value::<ImagePin>(pin.clone()).ok().map(|p| (name.clone(), p))).collect()
}

/// The pin for an image name such as `ubuntu-24.04`.
pub fn image(name: &str) -> Option<ImagePin> {
    all().into_iter().find(|(n, _)| n == name).map(|(_, p)| p)
}

/// The image names, in file order.
pub fn image_names() -> Vec<String> {
    all().into_iter().map(|(n, _)| n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pin_in_the_file_parses_with_a_well_formed_digest() {
        let value: serde_json::Value = serde_json::from_str(PINS).unwrap();
        let images = value["images"].as_object().unwrap();
        assert!(!images.is_empty());
        for (name, pin) in images {
            let parsed: ImagePin = serde_json::from_value(pin.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
            let hex = parsed.digest.strip_prefix("sha256:").unwrap();
            assert_eq!(hex.len(), 64, "{name}");
            assert!(hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "{name}");
            assert!(parsed.location.starts_with("https://"), "{name}: a pin is fetched over https");
        }
        assert_eq!(all().len(), images.len(), "a pin failed to load");
    }

    #[test]
    fn the_default_image_is_pinned() {
        assert!(image(DEFAULT_IMAGE).is_some());
        assert!(image("nonesuch").is_none());
        assert!(image_names().contains(&DEFAULT_IMAGE.to_owned()));
    }
}
