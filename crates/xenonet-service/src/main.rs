//! The service as a binary of its own, for its tests: `xenonet-service
//! [serve] [--demo] [--dev]`. XenoNet runs it as `xenonet serve`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = if args.first().map(String::as_str) == Some("serve") { &args[1..] } else { &args[..] };
    std::process::exit(xenonet_service::run(rest, env!("CARGO_PKG_VERSION")));
}
