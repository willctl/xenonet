//! The `eidolon` command. Every command takes `--json`. Exit 0 is done,
//! 1 is a refusal or a failure, 2 is a mistake in how it was typed. Errors
//! read `eidolon: <what happened> <what to do>` on standard error; with
//! `--json` they are a JSON object on standard output instead.
//!
//! `eidolon receive` and `eidolon mcp` own standard output: the move and the
//! protocol run over it, so their results and errors go to standard error.
//!
//! `eidolon mcp <eid> --http 127.0.0.1:<port>` serves the tools over HTTP
//! instead; it makes a token, writes it once to standard error, and stops
//! on Ctrl-C.
//!
//! `eidolon view` stays running while the person signs in. It writes the
//! address and the one-time secret to standard error (with `--json`, as one
//! JSON object on standard output with `"event": "open"`), waits for Enter
//! or Ctrl-C, closes the window, and then reports as the others do.
//!
//! - `args`: the command line.
//! - `home`, `store`: `$EIDOLON_HOME`, this machine's identity and keys.
//! - `cells`: a cell's wall (from Adytum's ledger) and its files (through
//!   Adytum's driver).
//! - `ops`: what each command does.

pub mod args;
pub mod browser;
pub mod cells;
pub mod error;
pub mod home;
pub mod ops;
pub mod store;

use args::{Cmd, Parsed};
use error::{CliError, Result};
use ops::Ctx;
use serde_json::Value;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::process::ExitCode;

/// Runs the command in `args` (without the program name). Results go to
/// `out`, errors in human form to `err`.
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> ExitCode {
    let parsed = match args::parse(args) {
        Ok(p) => p,
        Err(e) => return fail(&e, args.iter().any(|a| a == "--json"), out, err, false),
    };
    let json = parsed.json;
    let owns_stdout = matches!(parsed.cmd, Cmd::Receive | Cmd::Mcp { .. });
    let runtime = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
        Ok(r) => r,
        Err(e) => return fail(&CliError::io("start the runtime", &e), json, out, err, owns_stdout),
    };
    match runtime.block_on(dispatch(parsed, &mut *out, &mut *err)) {
        Ok(Done::Help) => {
            let _ = writeln!(out, "{}", args::USAGE);
            ExitCode::SUCCESS
        }
        Ok(Done::Silent) => ExitCode::SUCCESS,
        Ok(Done::Report(report)) => {
            let text = if json { report.json.to_string() } else { report.human };
            // A receiver's result must not reach standard output.
            let sink: &mut dyn Write = if owns_stdout { err } else { out };
            let _ = writeln!(sink, "{text}");
            let _ = sink.flush();
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e, json, out, err, owns_stdout),
    }
}

fn fail(e: &CliError, json: bool, out: &mut dyn Write, err: &mut dyn Write, owns_stdout: bool) -> ExitCode {
    if json && !owns_stdout {
        let _ = writeln!(out, "{}", e.json());
        let _ = out.flush();
    } else if json {
        let _ = writeln!(err, "{}", e.json());
    } else {
        let _ = writeln!(err, "{e}");
    }
    ExitCode::from(e.exit)
}

struct Report {
    json: Value,
    human: String,
}

enum Done {
    Help,
    /// The command already said all it has to say.
    Silent,
    Report(Report),
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).map_or_else(String::new, |x| x.as_str().map_or_else(|| x.to_string(), str::to_owned))
}

fn stdin_bytes() -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn cells_for(parsed: &Parsed) -> Result<Option<Box<dyn cells::Cells>>> {
    let needs = matches!(parsed.cmd, Cmd::Seal { .. } | Cmd::Open { .. } | Cmd::View { .. } | Cmd::Mcp { .. });
    if !needs {
        return Ok(None);
    }
    Ok(Some(match &parsed.cell_root {
        Some(root) => Box::new(cells::LocalCells::new(root.clone())),
        None => Box::new(cells::AdytumCells::system()?),
    }))
}

async fn dispatch(parsed: Parsed, out: &mut dyn Write, err: &mut dyn Write) -> Result<Done> {
    if parsed.cmd == Cmd::Help {
        return Ok(Done::Help);
    }
    let ctx = Ctx { home: home::Home::from_env()?, cells: cells_for(&parsed)? };
    let report = |json: Value, human: String| Ok(Done::Report(Report { json, human }));
    match &parsed.cmd {
        Cmd::Help => Ok(Done::Help),
        Cmd::MachineInit { name, key_store } => {
            let v = ops::machine_init(&ctx, name, key_store)?;
            let human = format!("{name} is set up. Its holder entry is `eidolon machine show`.\nwarning: {}", home::FILE_STORE_WARNING);
            report(v, human)
        }
        Cmd::MachineShow => {
            let holder = ops::machine_show(&ctx)?;
            let line = serde_json::to_value(&holder)?;
            report(line.clone(), line.to_string())
        }
        Cmd::New { wall, holders, facets } => {
            let v = ops::new(&ctx, wall, holders, facets, &mut stdin_bytes)?;
            let id = text(&v, "eidolon");
            report(v, id)
        }
        Cmd::Seal { eidolon, cell } => {
            let v = ops::seal_cmd(&ctx, eidolon, cell.as_deref()).await?;
            let human = format!("Sealed {eidolon} from {}: {} files, {} chunks, {}s.", text(&v, "cell"), text(&v, "files"), text(&v, "chunks"), text(&v, "seconds"));
            report(v, human)
        }
        Cmd::Open { eidolon, cell } => {
            let v = ops::open_cmd(&ctx, eidolon, cell).await?;
            let mut human = format!("Opened {eidolon} in {cell}.");
            if let Some(earlier) = v["cleared"].as_str() {
                let _ = write!(human, "\nAn earlier open had begun in {earlier} and not finished; the files it placed there are cleared.");
            }
            report(v, human)
        }
        Cmd::View { eidolon } => view(&ctx, eidolon, parsed.json, out, err).await,
        Cmd::Mcp { eidolon, http } => mcp(&ctx, eidolon, *http, err).await,
        Cmd::Move { eidolon, argv } => {
            let v = ops::move_cmd(&ctx, eidolon, argv).await?;
            let human =
                format!("Moved {eidolon}: {} of {} chunks sent, {} bytes, {}s.", text(&v, "chunks_sent"), text(&v, "chunks_offered"), text(&v, "bytes_sent"), text(&v, "seconds"));
            report(v, human)
        }
        Cmd::Receive => {
            let v = ops::receive_cmd(&ctx).await?;
            let human = format!("Received {} chunks of {} offered.", text(&v, "chunks_received"), text(&v, "chunks_offered"));
            report(v, human)
        }
        Cmd::Settle { eidolon, how } => {
            let v = ops::settle_cmd(&ctx, eidolon, how, &mut stdin_bytes)?;
            let human = format!("{eidolon} is {}.", text(&v, "settled"));
            report(v, human)
        }
        Cmd::Verify { eidolon } => {
            let v = ops::verify_cmd(&ctx, eidolon)?;
            let human = format!("{eidolon} is whole: {} chunks open.", text(&v, "chunks"));
            report(v, human)
        }
        Cmd::Facets { eidolon } => {
            let v = ops::facets_cmd(&ctx, eidolon)?;
            let rows: Vec<String> =
                v["facets"].as_array().map_or_else(Vec::new, |f| f.iter().map(|x| format!("{} {} {}", text(x, "name"), text(x, "kind"), text(x, "path"))).collect());
            report(v, rows.join("\n"))
        }
        Cmd::Ls => {
            let v = ops::ls_cmd(&ctx)?;
            let rows: Vec<String> = v["eidolons"].as_array().map_or_else(Vec::new, |f| {
                f.iter()
                    .map(|x| {
                        format!(
                            "{} {} {} generation {}{}",
                            text(x, "eidolon"),
                            text(x, "wall"),
                            x["state"],
                            text(x, "generation"),
                            if x["pending_move"] == true { " (move pending)" } else { "" }
                        )
                    })
                    .collect()
            });
            report(v, rows.join("\n"))
        }
    }
}

/// Waits until the command is interrupted (Ctrl-C, or the terminate signal).
async fn interrupted() {
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        () = term => {}
    }
}

/// Waits until the person presses Enter or the command is interrupted. A
/// closed standard input is not an answer, so it waits for the interrupt.
async fn wait_for_the_person() {
    use tokio::io::AsyncBufReadExt as _;
    let mut line = String::new();
    let mut input = tokio::io::BufReader::new(tokio::io::stdin());
    let enter = async {
        if matches!(input.read_line(&mut line).await, Ok(n) if n > 0) {
            return;
        }
        std::future::pending::<()>().await;
    };
    tokio::select! {
        () = enter => {}
        () = interrupted() => {}
    }
}

async fn view(ctx: &Ctx, eidolon: &eidolon_model::EidolonId, json: bool, out: &mut dyn Write, err: &mut dyn Write) -> Result<Done> {
    let (view, info) = ops::view_open(ctx, eidolon).await?;
    if json {
        let _ = writeln!(out, "{info}");
        let _ = out.flush();
    } else {
        let _ = writeln!(
            err,
            "The sign-in window for {eidolon} is open.\n  Address: {}\n  Secret:  {}\nOpen the address in Screen Sharing and sign in with the secret. The secret works for this session only.\nWhen you are done, press Enter here (or Ctrl-C). Until then the browser is not driven.",
            text(&info, "url"),
            text(&info, "secret")
        );
        let _ = err.flush();
    }
    wait_for_the_person().await;
    view.close().await?;
    let done = serde_json::json!({ "ok": true, "event": "closed", "eidolon": eidolon });
    Ok(Done::Report(Report { json: done, human: format!("Closed the window. The browser of {eidolon} is driven again.") }))
}

async fn mcp(ctx: &Ctx, eidolon: &eidolon_model::EidolonId, http: Option<std::net::SocketAddr>, err: &mut dyn Write) -> Result<Done> {
    let setup = ops::mcp_setup(ctx, eidolon).await?;
    match http {
        None => {
            let _ = writeln!(err, "eidolon: serving the browser of {eidolon} in {} on standard input and output. Files go through {}.", setup.cell, setup.files.display());
            let _ = err.flush();
            eidolon_mcp::serve_stdio(setup.tools).await?;
        }
        Some(addr) => {
            // Made fresh each run and shown once, here; it is the service's own
            // credential and nothing else keeps it.
            let bytes = xenonet_id::codec::random::<32>().map_err(|e| CliError::failed("random", e.to_string(), "Try again."))?;
            let token = xenonet_id::codec::b64(&bytes);
            let server = eidolon_mcp::HttpServer::bind(addr, &token).await?;
            let _ = writeln!(
                err,
                "eidolon: serving the browser of {eidolon} in {} at http://{}{}\n  Token: {token}\nSend it as `Authorization: Bearer <token>`. It is shown once. Files go through {}.\nPress Ctrl-C to stop.",
                setup.cell,
                server.local_addr()?,
                eidolon_mcp::PATH,
                setup.files.display()
            );
            let _ = err.flush();
            tokio::select! {
                served = server.serve(setup.tools) => served?,
                () = interrupted() => {}
            }
        }
    }
    Ok(Done::Silent)
}
