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
