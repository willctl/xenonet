//! Connect through every adapter in place: the registry's apps that have
//! connect, each through the format its adapter brings.

use serde_json::json;
use xenonet_adapter::connect::{Connect, FakeMachine, Instance, Options};

fn options(port: f64) -> Options {
    Options {
        port,
        demo: false,
        bin: "/x/xenonet".into(),
        home: "/h".into(),
        claude_config_dir: None,
        codex_home: None,
        grok_home: None,
        console_mcp: false,
        platform: "darwin".into(),
        instances: vec![Instance { id: "installed".into(), name: "XenoNet".into(), port: 47840.0 }, Instance { id: "dev".into(), name: "XenoNet Dev".into(), port: 47842.0 }],
    }
}

#[test]
fn the_adapters_connect_claude_code_codex_and_grok_build_through_their_formats() {
    xenonet_platforms::install();
    let c = Connect::new(options(47840.0));
    assert_eq!(c.ids(), ["claude", "codex", "grok"]);
    assert_eq!(c.names(), &("xenonet".to_string(), "xenonet-console".to_string()));
    let mut m = FakeMachine::default();
    let preview = c.preview(&mut m).unwrap();
    assert_eq!(preview["port"], json!(47840));
    assert_eq!(preview["demo"], json!(false));
    assert_eq!(preview["apps"].as_array().map(Vec::len), Some(3));
    // Grok Build isn't here, so connecting every app leaves it out.
    assert_eq!(c.installed_ids(&mut m).unwrap(), ["claude", "codex"]);
    let demo = Connect::new(Options { demo: true, ..options(47840.0) });
    assert_eq!(demo.apply(&mut m, &[]).unwrap_err().message, "this is the demo, so connect Claude Code, Codex and Grok Build from XenoNet, with your own sessions");
}
