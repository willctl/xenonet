//! The MCP servers as a binary of their own, for this crate's stdio tests;
//! XenoNet runs them as `xenonet mcp`, `xenonet mcp console` and `xenonet
//! mcp assistant`, and this takes the same arguments, with or without the
//! `mcp`, so the JS tests can run it as their xenonet too.

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let long = format!("--{name}");
    let eq = format!("--{name}=");
    args.iter().enumerate().find_map(|(i, a)| if *a == long { args.get(i + 1).map(String::as_str) } else { a.strip_prefix(&eq) })
}

// This binary is for the crate's tests, which check what the console
// operator lists and how it refuses arguments; driving XenoNet is the
// command's (xenonet mcp console).
struct NoControl;

impl xenonet_mcp::Control for NoControl {
    fn method(&self, _instance: &str, _kind: &str, _args: serde_json::Value) -> Result<serde_json::Value, String> {
        Err("this test binary drives no XenoNet: xenonet mcp console does".into())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "mcp") {
        args.remove(0);
    }
    let done = match args.first().map(String::as_str) {
        None => {
            xenonet_mcp::serve().await;
            Ok(())
        }
        Some("console") => xenonet_mcp::serve_console(std::sync::Arc::new(NoControl), false).await,
        Some("assistant") => {
            let port = flag(&args, "port")
                .and_then(|p| p.parse().ok())
                .or_else(|| args.iter().any(|a| a == "--dev").then_some(47842))
                .or_else(|| args.iter().any(|a| a == "--demo").then_some(47841));
            xenonet_mcp::serve_assistant(flag(&args, "grant").map(str::to_string), args.iter().any(|a| a == "--fixture"), port).await
        }
        Some(_) => Err("Use xenonet mcp, xenonet mcp console or xenonet mcp assistant.".into()),
    };
    if let Err(e) = done {
        eprintln!("xenonet: {e}");
        std::process::exit(1);
    }
}
