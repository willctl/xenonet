//! An agent drives the cell's browser through the facet's MCP tools, live.
//! Skipped unless `ADYTUM_LIVE=1`:
//!
//! ```text
//! ADYTUM_LIVE=1 npm run cargo -- test -p eidolon-browser --test agent -- --nocapture
//! ```
//!
//! It makes a cell named `adytum-test-agent-<tag>` under a short temporary
//! `ADYTUM_HOME`, provisions it, serves a test page inside the cell, and then
//! the test, standing in for the agent, takes a snapshot, fills, clicks,
//! uploads a file and downloads one, all through `BrowserTools`. It also asks
//! the first questions again through the MCP protocol, so the JSON shapes are
//! the real ones. The cell is removed also when a step fails.
#![expect(clippy::unwrap_used, reason = "a failing test should say where")]
#![expect(clippy::print_stdout, reason = "the test reports each step's time")]

use adytum_driver::{Driver, ExecOpts};
use adytum_lima::Lima;
use adytum_model::{CellName, CellSpec, DriverKind, Resources};
use eidolon_browser::units::Ports;
use eidolon_browser::{provision_plan, start_browser_plan, Browser, ConnectOptions};
use eidolon_mcp::{BrowserTools, Part};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PORT: u16 = 19223;
const VNC_PORT: u16 = 15901;
const DOWNLOAD_LEN: usize = 200 * 1024;

/// The test page's server, written into the guest. Python 3 is on the image.
const SERVER: &str = r#"
import hashlib
from email.parser import BytesParser
from email.policy import HTTP
from http.server import BaseHTTPRequestHandler, HTTPServer

PAGE = b"""<!doctype html><html><head><title>agent test</title></head><body>
<h1>Agent test</h1>
<input id="name" type="text" aria-label="Name">
<button id="go" type="button" onclick="document.getElementById('result').textContent = document.getElementById('name').value">Show</button>
<p id="result"></p>
<form method="post" action="/upload" enctype="multipart/form-data">
<input id="file" type="file" name="f" aria-label="Attachment"><button type="submit">Send</button>
</form>
<a href="/download">Get report</a>
</body></html>"""

def content():
    return bytes((i * 31 + 7) & 255 for i in range(%LEN%))

class H(BaseHTTPRequestHandler):
    def reply(self, code, body, kind, extra=()):
        self.send_response(code)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        for k, v in extra:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/":
            self.reply(200, PAGE, "text/html")
        elif self.path == "/download":
            self.reply(200, content(), "text/csv", [("Content-Disposition", "attachment; filename=report.csv")])
        else:
            self.reply(404, b"no", "text/plain")

    def do_POST(self):
        raw = self.rfile.read(int(self.headers["Content-Length"]))
        head = ("Content-Type: " + self.headers["Content-Type"] + "\r\n\r\n").encode()
        message = BytesParser(policy=HTTP).parsebytes(head + raw)
        part = next(message.iter_parts())
        data = part.get_payload(decode=True)
        text = "uploaded %d bytes sha256=%s" % (len(data), hashlib.sha256(data).hexdigest())
        self.reply(200, ("<!doctype html><title>done</title><p>" + text + "</p>").encode(), "text/html")

HTTPServer(("127.0.0.1", 8000), H).serve_forever()
"#;

fn tag() -> String {
    format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()) & 0xffff_ffff)
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_owned()).collect()
}

fn expected_download() -> Vec<u8> {
    (0..DOWNLOAD_LEN).map(|i| ((i * 31 + 7) & 255) as u8).collect()
}

fn hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

type Res = Result<(), Box<dyn std::error::Error>>;

struct Steps(Vec<(String, f64)>);

impl Steps {
    async fn run<T>(&mut self, what: &str, work: impl std::future::Future<Output = T>) -> T {
        let start = Instant::now();
        let out = work.await;
        let secs = start.elapsed().as_secs_f64();
        println!("{what}: {secs:.1}s");
        self.0.push((what.to_owned(), secs));
        out
    }
}

fn text_of(parts: &[Part]) -> String {
    parts.iter().filter_map(|p| if let Part::Text(t) = p { Some(t.as_str()) } else { None }).collect::<Vec<_>>().join("\n")
}

async fn call(tools: &BrowserTools, name: &str, args: Value) -> Result<String, Box<dyn std::error::Error>> {
    Ok(text_of(&tools.call_tool(name, args).await?))
}

/// The uid of the snapshot line holding `needle`.
fn uid_of(snapshot: &str, needle: &str) -> String {
    let line = snapshot.lines().find(|l| l.contains(needle)).unwrap_or_default();
    assert!(!line.is_empty(), "no {needle:?} in the snapshot:\n{snapshot}");
    let uid = line.split('[').nth(1).and_then(|s| s.split(']').next()).unwrap_or("-");
    assert_ne!(uid, "-", "{needle:?} has no uid:\n{snapshot}");
    uid.to_owned()
}

/// Asks `check` again and again, with a pause, until it says it is done.
/// The attempts are bounded; what `check` saw last is in the failure.
async fn until<F, Fut>(what: &str, attempts: u32, mut check: F) -> Result<String, Box<dyn std::error::Error>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<String>, Box<dyn std::error::Error>>>,
{
    for _ in 0..attempts {
        if let Some(found) = check().await? {
            return Ok(found);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(format!("{what} did not happen within {attempts} tries").into())
}

async fn site(lima: &Lima, name: &CellName) -> Res {
    let server = SERVER.replace("%LEN%", &DOWNLOAD_LEN.to_string());
    let opts = ExecOpts { stdin: Some(server.into_bytes()), ..ExecOpts::default() };
    let made = lima.exec(name, &strings(&["sudo", "install", "-d", "-m", "0755", "/opt/testpage"]), ExecOpts::default()).await?;
    assert!(made.success(), "{}", made.stderr_lossy());
    let wrote = lima.exec(name, &strings(&["sudo", "tee", "/opt/testpage/server.py"]), opts).await?;
    assert!(wrote.success(), "{}", wrote.stderr_lossy());
    let serve = lima
        .exec(name, &strings(&["sudo", "systemd-run", "--unit=eidolon-test-page", "--working-directory=/opt/testpage", "python3", "/opt/testpage/server.py"]), ExecOpts::default())
        .await?;
    assert!(serve.success(), "{}", serve.stderr_lossy());
    Ok(())
}

async fn scenario(lima: Arc<Lima>, name: &CellName, home: &Path, steps: &mut Steps) -> Res {
    let spec = CellSpec {
        name: name.clone(),
        driver: DriverKind::Lima,
        image: adytum_lima::image(adytum_lima::DEFAULT_IMAGE).unwrap(),
        wall: "wall_test".parse()?,
        resources: Resources { cpus: 2, memory_mib: 3072, disk_gib: 10 },
        ports: vec![],
        automation_port: Some(PORT),
        mounts: vec![],
        sealed: true,
        network: None,
        clone_from: None,
        secrets: vec![],
    };
    steps.run("create", lima.create(&spec)).await?;
    steps.run("start", lima.start(name)).await?;
    let ports = Ports { automation: PORT, vnc: VNC_PORT };
    steps.run("provision", provision_plan(ports).run(&*lima, name)).await?;
    steps.run("serve the test page", site(&lima, name)).await?;
    steps.run("start browser", start_browser_plan().run(&*lima, name)).await?;

    let files = home.join("files");
    std::fs::create_dir_all(&files)?;
    let upload: Vec<u8> = (0..70_000u32).map(|i| (i.wrapping_mul(17) % 251) as u8).collect();
    std::fs::write(files.join("notes.bin"), &upload)?;

    let driver: Arc<dyn Driver> = lima.clone();
    let browser = Arc::new(Browser::supervise(driver.clone(), name.clone(), PORT, ConnectOptions::default()));
    let tools = Arc::new(BrowserTools::new(browser.clone(), driver, name.clone(), files.clone()));

    steps.run("first page", async { browser.session().await }).await?;
    let went = steps.run("navigate", call(&tools, "navigate_page", json!({ "url": "http://127.0.0.1:8000/" }))).await?;
    assert!(went.contains("Loaded"), "{went}");

    let snapshot = steps.run("snapshot", call(&tools, "take_snapshot", json!({}))).await?;
    println!("{snapshot}");
    let field = uid_of(&snapshot, "textbox");
    let button = uid_of(&snapshot, "button \"Show\"");
    let file = uid_of(&snapshot, "file");
    assert_ne!(uid_of(&snapshot, "link \"Get report\""), "-");

    steps
        .run("fill and click", async {
            call(&tools, "fill_by_uid", json!({ "uid": field, "value": "hello agent" })).await?;
            call(&tools, "click_by_uid", json!({ "uid": button })).await
        })
        .await?;
    let text = call(&tools, "get_page_text", json!({})).await?;
    assert!(text.contains("hello agent"), "{text}");

    // Upload: the file is in the host folder; the page's server tells what it got.
    let uploaded = steps
        .run("upload", async {
            let said = call(&tools, "upload_file_by_uid", json!({ "uid": file, "filePath": files.join("notes.bin").to_string_lossy() })).await?;
            assert!(said.contains("notes.bin"), "{said}");
            let submit = call(&tools, "take_snapshot", json!({})).await?;
            call(&tools, "click_by_uid", json!({ "uid": uid_of(&submit, "button \"Send\"") })).await?;
            until("the upload's answer", 20, || async { Ok(Some(call(&tools, "get_page_text", json!({})).await?).filter(|t| t.contains("sha256="))) }).await
        })
        .await?;
    assert!(uploaded.contains(&format!("uploaded {} bytes sha256={}", upload.len(), hex(&upload))), "{uploaded}");

    // Download: back on the page, click the link, wait until the file is complete.
    call(&tools, "navigate_page", json!({ "url": "http://127.0.0.1:8000/" })).await?;
    let again = call(&tools, "take_snapshot", json!({})).await?;
    let link2 = uid_of(&again, "link \"Get report\"");
    let fetched = steps
        .run("download", async {
            call(&tools, "click_by_uid", json!({ "uid": link2 })).await?;
            let done = format!("report.csv {DOWNLOAD_LEN} bytes\n");
            until("the download", 30, || async { Ok(Some(call(&tools, "list_downloads", json!({})).await?).filter(|l| l.contains(&done))) }).await?;
            call(&tools, "fetch_download", json!({ "name": "report.csv" })).await
        })
        .await?;
    println!("{fetched}");
    let got = std::fs::read(files.join("downloads/report.csv"))?;
    assert_eq!(got, expected_download(), "the host copy differs from what the server sent");

    let png = call_image(&tools).await?;
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

    steps.run("the same over the protocol", over_protocol(tools)).await?;

    // Neither port is reachable from the host.
    for port in [PORT, VNC_PORT] {
        let addr: SocketAddr = ([127, 0, 0, 1], port).into();
        assert!(TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_err(), "port {port} reached the host");
    }
    not_forwarded(home, name, &[PORT, VNC_PORT])?;
    Ok(())
}

async fn call_image(tools: &BrowserTools) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let parts = tools.call_tool("screenshot_page", json!({})).await?;
    let Some(Part::Image { data, mime }) = parts.into_iter().next() else { return Err("screenshot_page gave no image".into()) };
    assert_eq!(mime, "image/png");
    Ok(decode_base64(&data))
}

fn decode_base64(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut bits = 0u32;
    let mut n = 0;
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        bits = (bits << 6) | u32::from(v);
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push(((bits >> n) & 255) as u8);
        }
    }
    out
}

/// The tools once more, spoken to as an MCP client over an in-memory duplex.
async fn over_protocol(tools: Arc<BrowserTools>) -> Res {
    let (client, server) = tokio::io::duplex(1 << 22);
    // `serve_transport` takes the tools by value; the clone-free route is a
    // second set built by the caller, so this runs on the one it is given.
    let tools = Arc::try_unwrap(tools).map_err(|_| "the tools are still shared")?;
    tokio::spawn(eidolon_mcp::serve_transport(tools, tokio::io::split(server)));
    let (read, mut write) = tokio::io::split(client);
    let mut read = BufReader::new(read);
    let mut id = 0u64;
    let mut request = async |method: &str, params: Value| -> Result<Value, Box<dyn std::error::Error>> {
        id += 1;
        write.write_all(format!("{}\n", json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).as_bytes()).await?;
        write.flush().await?;
        loop {
            let mut line = String::new();
            if read.read_line(&mut line).await? == 0 {
                return Err("the server ended".into());
            }
            let message: Value = serde_json::from_str(&line)?;
            if message["id"] == json!(id) {
                return Ok(message);
            }
        }
    };
    let init = request("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } })).await?;
    assert_eq!(init["result"]["serverInfo"]["name"], "eidolon-browser", "{init}");
    let snapshot = request("tools/call", json!({ "name": "take_snapshot", "arguments": {} })).await?;
    let text = snapshot["result"]["content"][0]["text"].as_str().unwrap_or_default().to_owned();
    assert!(text.contains("link \"Get report\""), "{snapshot}");
    let shot = request("tools/call", json!({ "name": "screenshot_page", "arguments": {} })).await?;
    assert_eq!(shot["result"]["content"][0]["mimeType"], "image/png", "{shot}");
    Ok(())
}

/// Lima's own record of the cell shows no forward for the ports: its rules
/// only ignore them, and its host agent's log never says it forwarded one.
fn not_forwarded(home: &Path, name: &CellName, ports: &[u16]) -> Res {
    let lima_home = adytum_lima::lima_home_in(home);
    let out = std::process::Command::new("limactl").args(["list", "--json"]).env("LIMA_HOME", &lima_home).output()?;
    assert!(out.status.success());
    let listing = String::from_utf8(out.stdout)?;
    let mut seen = false;
    for line in listing.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line)?;
        if v["name"] != name.as_str() {
            continue;
        }
        seen = true;
        for rule in v["config"]["portForwards"].as_array().cloned().unwrap_or_default() {
            if rule["ignore"] == json!(true) {
                continue;
            }
            for port in ports {
                assert_ne!(rule["guestPort"], json!(port), "a forward rule names port {port}: {rule}");
                assert_ne!(rule["hostPort"], json!(port), "a forward rule names host port {port}: {rule}");
            }
        }
    }
    assert!(seen, "the cell is not in limactl's list");
    let log = std::fs::read_to_string(lima_home.join(name.as_str()).join("ha.stderr.log")).unwrap_or_default();
    for port in ports {
        let at = format!(":{port}");
        assert!(!log.lines().any(|l| l.to_ascii_lowercase().contains("forwarding") && l.contains(&at)), "the host agent forwarded port {port}");
    }
    Ok(())
}

fn tmp_home() -> PathBuf {
    PathBuf::from("/tmp").join(format!("eag-{}", tag()))
}

#[tokio::test]
async fn an_agent_drives_the_browser_in_a_real_cell() {
    if std::env::var("ADYTUM_LIVE").as_deref() != Ok("1") {
        println!("skipped: set ADYTUM_LIVE=1 to run against a real VM");
        return;
    }
    let home = tmp_home();
    std::fs::create_dir_all(&home).unwrap();
    let lima = Arc::new(Lima::system(adytum_lima::lima_home_in(&home)));
    let name: CellName = format!("adytum-test-agent-{}", tag()).parse().unwrap();
    let mut steps = Steps(Vec::new());
    let result = scenario(lima.clone(), &name, &home, &mut steps).await;
    let removed = lima.remove(&name).await;
    let _ = std::fs::remove_dir_all(&home);
    println!("--- seconds per step ---");
    for (what, secs) in &steps.0 {
        println!("{secs:7.1}  {what}");
    }
    removed.unwrap();
    result.unwrap();
}

/// The tools this test calls exist, so a rename is caught without a VM.
#[test]
fn the_tools_the_agent_uses_exist() {
    let names: Vec<&str> = eidolon_mcp::definitions().iter().map(|d| d.name).collect();
    for needed in ["navigate_page", "take_snapshot", "fill_by_uid", "click_by_uid", "get_page_text", "upload_file_by_uid", "list_downloads", "fetch_download", "screenshot_page"] {
        assert!(names.contains(&needed), "{needed} is not a tool of eidolon-mcp");
    }
}
