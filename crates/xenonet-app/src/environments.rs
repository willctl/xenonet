//! core/environments.js: the place around the map, chosen apart from its
//! theme.

pub struct Environment {
    pub id: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    pub shape: &'static str,
    pub index: u32,
}

pub const DEFAULT_ENVIRONMENT: &str = "nebula";

pub const ENVIRONMENTS: [Environment; 4] = [
    Environment { id: "nebula", name: "Nebula", about: "The original drifting clouds and star field.", shape: "clouds", index: 0 },
    Environment { id: "aurora", name: "Aurora", about: "Slow curtains of light across a quiet atmospheric horizon.", shape: "curtains", index: 1 },
    Environment { id: "trench", name: "Trench", about: "A deep-sea trench with rippling caustics and distant light shafts.", shape: "water", index: 2 },
    Environment { id: "vault", name: "Vault", about: "A monumental hall of repeating arches and illuminated ribs.", shape: "arches", index: 3 },
];

/// The place with this id, or the first one.
pub fn environment_of(id: Option<&str>) -> &'static Environment {
    id.and_then(|id| ENVIRONMENTS.iter().find(|e| e.id == id)).unwrap_or(&ENVIRONMENTS[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_place_or_the_first() {
        assert_eq!(environment_of(Some("vault")).name, "Vault");
        assert_eq!(environment_of(Some("nowhere")).id, DEFAULT_ENVIRONMENT);
        assert_eq!(environment_of(None).id, "nebula");
        assert!(ENVIRONMENTS.iter().enumerate().all(|(i, e)| e.index as usize == i));
    }
}
