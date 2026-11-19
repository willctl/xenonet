//! The cell every test uses.

use adytum_model::{CellName, CellSpec, DriverKind, Resources};

pub const NAME: &str = "adytum-test-golden";

pub fn name() -> CellName {
    NAME.parse().expect("a valid name")
}

pub fn spec() -> CellSpec {
    CellSpec {
        name: name(),
        driver: DriverKind::Lima,
        image: adytum_lima::image(adytum_lima::DEFAULT_IMAGE).expect("the default image is pinned"),
        wall: "wall_test".parse().expect("a valid wall"),
        resources: Resources::default(),
        ports: vec![],
        automation_port: None,
        mounts: vec![],
        sealed: true,
        network: None,
        clone_from: None,
        secrets: vec![],
    }
}

pub const CLONE_NAME: &str = "adytum-test-golden-clone";

pub fn clone_name() -> CellName {
    CLONE_NAME.parse().expect("a valid name")
}

/// The clone of [`spec`]: same wall, a port of its own.
pub fn clone_spec() -> CellSpec {
    CellSpec { name: clone_name(), clone_from: Some(name()), ports: vec![adytum_model::Port { guest: 8080, host: 18080, bind: std::net::IpAddr::from([127, 0, 0, 1]) }], ..spec() }
}
