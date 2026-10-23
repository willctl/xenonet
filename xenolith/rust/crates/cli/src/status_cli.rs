//! `xenolith status [--port N] [--json]`: is the service healthy, and is the audit chain being
//! written. The service's /status (on the port its owner key names, else config.json's httpPort,
//! else the default), and this machine's audit chain: whether it holds, and whether a line can be
//! added. Exits 1 when the service is down or degraded, or the chain is broken or can't be written.

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use xenolith_core::audit;
use xenolith_core::paths::AppPaths;

fn get_status(port: u16) -> Result<Value, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    write!(s, "GET /status HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n").map_err(|e| e.to_string())?;
    let mut raw = String::new();
    s.read_to_string(&mut raw).map_err(|e| e.to_string())?;
    let body = raw.split_once("\r\n\r\n").map(|(_, b)| b).ok_or("malformed response")?;
    serde_json::from_str(body).map_err(|_| "not JSON".to_string())
}

pub fn run(args: &[String]) -> ! {
    let json_out = args.iter().any(|a| a == "--json");
    let paths = AppPaths::current();
    let port = args
        .windows(2)
        .find(|w| w[0] == "--port")
        .and_then(|w| w[1].parse::<u16>().ok())
        .or_else(|| xenolith_core::owner::read(&paths).map(|k| k.port))
        .or_else(|| xenolith_core::wiki::read_config(&crate::mcp::env()).ok().and_then(|c| c.get("httpPort").and_then(Value::as_u64)).and_then(|p| u16::try_from(p).ok()))
        .unwrap_or(xenolith_core::wiki::DEFAULT_HTTP_PORT);
    let service = get_status(port);
    let file = audit::chain_file(&paths);
    let verified = audit::verify(&file);
    let writable = audit::writable(&paths);
    let service_audit = service.as_ref().ok().map(|s| s["audit"].clone()).unwrap_or(Value::Null);
    let healthy = service.as_ref().is_ok_and(|s| s["health"] == "ok");
    let chain_ok = verified.as_ref().is_ok_and(|v| v.broken.is_none()) && writable.is_ok() && service_audit["ok"] != false;
    if json_out {
        println!(
            "{}",
            json!({
                "port": port,
                "service": service.as_ref().ok(),
                "serviceError": service.as_ref().err(),
                "audit": {
                    "file": xenolith_core::wiki::disp(&file),
                    "lines": verified.as_ref().ok().map(|v| v.lines),
                    "head": verified.as_ref().ok().map(|v| v.head.clone()),
                    "broken": verified.as_ref().ok().and_then(|v| v.broken.as_ref()).map(|(n, why)| json!({ "line": n, "why": why })),
                    "error": verified.as_ref().err().map(|e| e.to_string()),
                    "writable": writable.is_ok(),
                    "writeError": writable.as_ref().err(),
                    "service": service_audit,
                    "ok": chain_ok,
                },
            })
        );
    } else {
        match &service {
            Ok(s) => {
                println!("Service: {} (xenolith {}, pid {}, http://127.0.0.1:{port}/mcp)", s["health"].as_str().unwrap_or("unknown"), s["version"].as_str().unwrap_or("?"), s["pid"]);
                for r in s["reasons"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    println!("  ! {r}");
                }
            }
            Err(e) => println!("Service: not answering on port {port} ({e})"),
        }
        match (&verified, &writable) {
            (Err(e), _) => println!("Audit chain: ! cannot be read: {e}"),
            (Ok(v), _) if v.broken.is_some() => {
                let (line, why) = v.broken.as_ref().unwrap();
                println!("Audit chain: ! breaks at line {line} of {}: {why}", xenolith_core::wiki::disp(&file));
            }
            (Ok(_), Err(e)) => println!("Audit chain: ! can't be written: {e}"),
            (Ok(v), Ok(())) => println!("Audit chain: holds, {} entries, last hash {}", v.lines, v.head),
        }
    }
    std::process::exit(if healthy && chain_ok { 0 } else { 1 })
}
