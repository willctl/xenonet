//! The browser in a real Lima VM. Skipped unless `ADYTUM_LIVE=1`:
//!
//! ```text
//! ADYTUM_LIVE=1 npm run cargo -- test -p eidolon-browser --test live -- --nocapture
//! ```
//!
//! It makes a cell named `adytum-test-browser-<tag>` under a short temporary
//! `ADYTUM_HOME`, provisions it, drives a page served inside the cell, proves
//! the automation port is not reachable from the host, and removes the cell
//! also when a step fails.
#![expect(clippy::unwrap_used, reason = "a failing test should say where")]
#![expect(clippy::print_stdout, reason = "the test reports each step's time")]

use adytum_driver::{Driver, ExecOpts};
use adytum_lima::Lima;
use adytum_model::{CellName, CellSpec, DriverKind, Resources};
use eidolon_bidi::Wait;
use eidolon_browser::units::Ports;
use eidolon_browser::{provision_plan, start_browser_plan, Browser, ConnectOptions};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

const PORT: u16 = 19222;
const PAGE: &str = r#"<!doctype html><html><head><title>start</title></head><body>
<input id="name" type="text"><button id="go" type="button" onclick="document.cookie='seen=1; max-age=31536000; path=/'; document.title='clicked'">Go</button>
</body></html>"#;

fn tag() -> String {
    format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()) & 0xffff_ffff)
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_owned()).collect()
}

async fn timed<T>(what: &str, work: impl std::future::Future<Output = T>) -> T {
    let start = Instant::now();
    let out = work.await;
    println!("{what}: {:.1}s", start.elapsed().as_secs_f64());
    out
}

type Res = Result<(), Box<dyn std::error::Error>>;

async fn scenario(lima: &Lima, name: &CellName) -> Res {
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
    timed("create", lima.create(&spec)).await?;
    timed("start", lima.start(name)).await?;
    timed("provision", provision_plan(Ports { automation: PORT, ..Ports::default() }).run(lima, name)).await?;

    // A page served inside the cell, on its loopback only.
    let site = ExecOpts { stdin: Some(PAGE.as_bytes().to_vec()), ..ExecOpts::default() };
    lima.exec(name, &strings(&["mkdir", "-p", "/tmp/site"]), ExecOpts::default()).await?;
    lima.exec(name, &strings(&["tee", "/tmp/site/index.html"]), site).await?;
    let serve = lima
        .exec(
            name,
            &strings(&["sudo", "systemd-run", "--unit=eidolon-test-site", "--working-directory=/tmp/site", "python3", "-m", "http.server", "8000", "--bind", "127.0.0.1"]),
            ExecOpts::default(),
        )
        .await?;
    assert!(serve.success(), "{}", serve.stderr_lossy());

    timed("start browser", start_browser_plan().run(lima, name)).await?;
    let session = timed("connect", Browser::connect_with(lima, name, PORT, ConnectOptions::default())).await?;
    session.new_session().await?;
    let context = session.browsing_context_create().await?;
    timed("navigate", session.navigate(&context, "http://127.0.0.1:8000/", Wait::Complete)).await?;
    let field = session.locate_nodes(&context, "#name").await?;
    session.click(&context, &field[0].shared_id).await?;
    session.type_text(&context, "hello").await?;
    let button = session.locate_nodes(&context, "#go").await?;
    session.click(&context, &button[0].shared_id).await?;
    let value = session.script_evaluate(&context, "document.getElementById('name').value + '|' + document.title + '|' + document.cookie", false).await?;
    let text = value.pointer("/value").and_then(|v| v.as_str()).unwrap_or_default().to_owned();
    assert_eq!(text, "hello|clicked|seen=1");
    let png = session.capture_screenshot(&context).await?;
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

    // The automation port exists only inside the cell.
    let addr: SocketAddr = ([127, 0, 0, 1], PORT).into();
    assert!(TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_err(), "the automation port reached the host");
    Ok(())
}

#[tokio::test]
async fn a_browser_in_a_real_cell() {
    if std::env::var("ADYTUM_LIVE").as_deref() != Ok("1") {
        println!("skipped: set ADYTUM_LIVE=1 to run against a real VM");
        return;
    }
    let home = std::path::PathBuf::from("/tmp").join(format!("adbr-{}", tag()));
    let lima = Lima::system(adytum_lima::lima_home_in(&home));
    let name: CellName = format!("adytum-test-browser-{}", tag()).parse().unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let result = scenario(&lima, &name).await;
    let removed = lima.remove(&name).await;
    let _ = std::fs::remove_dir_all(&home);
    removed.unwrap();
    result.unwrap();
}
