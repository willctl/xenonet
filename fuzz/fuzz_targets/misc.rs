#![no_main]

// The smaller readers of what other programs send: a component package
// (.tar.gz) unpacked and its JSON-RPC lines, a remote actor's read
// requests and the cursors it hands back, the tunnel client's output and
// profiles, actor and gateway tokens, a Windows file's ACL, a theme file's
// overrides, and base64. The first byte picks one.

use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use xenonet_assistant::cursors::{Cursors, For};
use xenonet_assistant::service::{self, Config};
use xenonet_assistant::{crypto, tunnel_client};
use xenonet_components::rpc::Rpc;
use xenonet_components::{packages, protocol, tar};

const NOW: f64 = 1_791_230_000_000.0;

fn dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| std::env::temp_dir().join(format!("xenonet-fuzz-misc-{}", std::process::id())))
}

fn config() -> Config {
    Config {
        operations: vec![("xenonet_overview".into(), "overview".into()), ("xenonet_sessions".into(), "sessions".into()), ("xenonet_session".into(), "session".into())],
        attention_kinds: vec!["question".into()],
        result_schema: "assistant/result/v1".into(),
        error_schema: "assistant/error/v1".into(),
        version: "0.0.0".into(),
        limits: service::LIMITS,
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else { return };
    let text = xenonet_fuzz::text(rest);
    let v = xenonet_fuzz::json(rest);
    xenonet_fuzz::run(rest, || match pick % 6 {
        0 => {
            // Unpacked into a folder of its own, gone after.
            let d = dir();
            let _ = std::fs::remove_dir_all(d);
            std::fs::create_dir_all(d).expect("a temp folder");
            let _ = tar::unpack(rest, d);
            let _ = std::fs::remove_dir_all(d);
        }
        1 => {
            let mut rpc = Rpc::new(1 << 16, vec!["progress".into(), "log".into()]);
            let cut = usize::from(pick / 6) % (text.len() + 1);
            let cut = (0..=cut).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
            let _ = rpc.data(&text[..cut]);
            let _ = rpc.data(&text[cut..]);
            let _ = rpc.data("\n");
            let _ = protocol::negotiate(Some(&json!([1, 1])), v.as_ref());
            let _ = packages::package_info(v.as_ref(), "darwin", "arm64");
            let _ = packages::newer(v.as_ref().and_then(|v| v.get("a")), v.as_ref().and_then(|v| v.get("b")));
        }
        2 => {
            let cfg = config();
            let b = service::begin(&cfg, v.as_ref().unwrap_or(&Value::Null));
            let _ = (b.request_id, b.op, b.tool, b.args);
            let _ = service::is_request_id(&text);
            // A cursor as the actor hands it back: read under this key, and
            // one this key made, with the input as its filters.
            let cursors = Cursors::new(Some(vec![7; 32]));
            let grant = json!("grant-1");
            let f = For { grant: Some(&grant), revision: Some(&json!(3)), op: Some(&json!("sessions")), filters: v.as_ref() };
            let _ = cursors.read(&text, &f, NOW);
            let made = cursors.make(&f, v.as_ref(), NOW);
            let _ = cursors.read(&made, &f, NOW + 1000.0);
            let _ = crypto::b64url_decode(&text);
        }
        3 => {
            let _ = (tunnel_client::read_profile(&text), tunnel_client::log_line(&text), tunnel_client::version_in(&text), tunnel_client::mask_keys(&text));
            let _ = (tunnel_client::is_tunnel_id(&text), tunnel_client::is_runtime_key(&text));
            let _ = tunnel_client::health_of(v.as_ref());
            if let Some(Value::Array(argv)) = &v {
                let _ = tunnel_client::command_line(argv);
            }
        }
        4 => {
            let (a, b) = text.split_once('\0').unwrap_or((&text, ""));
            let _ = (xenonet_actors::tokens::is_token(a), xenonet_actors::tokens::is_gateway_token(a), xenonet_actors::tokens::bearer_of(a));
            let _ = (xenonet_actors::secret_store::is_target(a), xenonet_actors::secret_store::is_secret(a));
            let _ = xenonet_base::private_file::others_in(a, b);
            let _ = xenonet_actors::token_file::token_file_refusal(a, false, Some("/home/nova/Projects"));
        }
        _ => {
            if let Some(Value::Object(custom)) = &v {
                let _ = xenonet_app::themes::themes(custom);
            }
            let _ = xenonet_app::themes::merge(v.as_ref(), v.as_ref().and_then(|v| v.get("over")));
            let _ = xenonet_integrations::b64::decode(&text);
            let _ = xenonet_integrations::b64::byte_length(&text);
        }
    });
});
