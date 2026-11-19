//! The window's rules, against a fake driver whose stream is a tiny fake
//! VNC server.
#![expect(clippy::unwrap_used, reason = "a failing test should say where; helpers here are not #[test] fns")]

use adytum_driver::{Driver, DriverError, ExecOpts, Listed};
use adytum_exec::{Duplex, DuplexControl, ExecError, Output};
use adytum_model::{CellName, CellSpec};
use async_trait::async_trait;
use eidolon_browser::units::Ports;
use eidolon_view::{OpenOptions, View, ViewError, ViewGuard};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn cell() -> CellName {
    "adytum-test-view".parse().unwrap()
}

#[derive(Clone)]
struct Exec {
    argv: Vec<String>,
    stdin: Option<String>,
}

#[derive(Default)]
struct FakeDriver {
    execs: Mutex<Vec<Exec>>,
    streams: Mutex<Vec<Vec<String>>>,
}

impl FakeDriver {
    fn execs(&self) -> Vec<Exec> {
        self.execs.lock().unwrap().clone()
    }
    fn stream_argvs(&self) -> Vec<Vec<String>> {
        self.streams.lock().unwrap().clone()
    }
}

struct NoControl;

#[async_trait]
impl DuplexControl for NoControl {
    async fn finish(self: Box<Self>) -> Result<Output, ExecError> {
        Ok(Output::default())
    }
    fn kill(&mut self) {}
}

/// Sends the RFB banner, then echoes what it is sent.
async fn fake_vnc(mut end: tokio::io::DuplexStream) {
    if end.write_all(b"RFB 003.008\n").await.is_err() {
        return;
    }
    let mut buf = [0u8; 256];
    while let Ok(n) = end.read(&mut buf).await {
        if n == 0 || end.write_all(&buf[..n]).await.is_err() {
            return;
        }
    }
}

fn unsupported(what: &str) -> DriverError {
    DriverError::Unsupported { what: what.to_owned() }
}

#[async_trait]
impl Driver for FakeDriver {
    async fn create(&self, _: &CellSpec) -> Result<(), DriverError> {
        Err(unsupported("create"))
    }
    async fn start(&self, _: &CellName) -> Result<(), DriverError> {
        Err(unsupported("start"))
    }
    async fn exec(&self, _: &CellName, argv: &[String], opts: ExecOpts) -> Result<Output, DriverError> {
        let stdin = opts.stdin.map(|b| String::from_utf8_lossy(&b).into_owned());
        self.execs.lock().unwrap().push(Exec { argv: argv.to_vec(), stdin });
        Ok(Output { status: Some(0), ..Output::default() })
    }
    async fn stream(&self, _: &CellName, argv: &[String]) -> Result<Duplex, DriverError> {
        self.streams.lock().unwrap().push(argv.to_vec());
        let (client, server) = tokio::io::duplex(1 << 16);
        tokio::spawn(fake_vnc(server));
        let (read, write) = tokio::io::split(client);
        Ok(Duplex::from_parts(Box::new(read), Box::new(write), Box::new(NoControl)))
    }
    async fn copy_in(&self, _: &CellName, _: &Path, _: &str) -> Result<(), DriverError> {
        Err(unsupported("copy_in"))
    }
    async fn copy_out(&self, _: &CellName, _: &str, _: &Path) -> Result<(), DriverError> {
        Err(unsupported("copy_out"))
    }
    async fn stop(&self, _: &CellName) -> Result<(), DriverError> {
        Err(unsupported("stop"))
    }
    async fn remove(&self, _: &CellName) -> Result<(), DriverError> {
        Err(unsupported("remove"))
    }
    async fn list(&self) -> Result<Vec<Listed>, DriverError> {
        Ok(Vec::new())
    }
}

/// A scratch folder that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("eidolon-view-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ports() -> Ports {
    Ports { automation: 9222, vnc: 5901 }
}

async fn open(driver: &Arc<FakeDriver>) -> View {
    View::open(driver.clone(), cell(), ports()).await.unwrap()
}

async fn banner(addr: std::net::SocketAddr) -> (TcpStream, String) {
    let mut sock = TcpStream::connect(addr).await.unwrap();
    let mut got = [0u8; 12];
    sock.read_exact(&mut got).await.unwrap();
    (sock, String::from_utf8(got.to_vec()).unwrap())
}

#[tokio::test]
async fn the_listener_is_on_127_0_0_1_only() {
    let driver = Arc::new(FakeDriver::default());
    let view = open(&driver).await;
    assert_eq!(view.address().ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert_ne!(view.address().port(), 0);
    assert_eq!(view.url(), format!("vnc://127.0.0.1:{}", view.address().port()));
    view.close().await.unwrap();
}

#[tokio::test]
async fn binding_anywhere_but_127_0_0_1_is_refused_before_the_cell_is_touched() {
    for bad in ["0.0.0.0", "192.168.1.5", "::1", "::"] {
        let driver = Arc::new(FakeDriver::default());
        let options = OpenOptions { bind: bad.parse().unwrap(), ..OpenOptions::default() };
        let err = View::open_with(driver.clone(), cell(), ports(), options).await.unwrap_err();
        assert!(matches!(err, ViewError::NotLoopback { .. }), "{bad}: {err}");
        assert!(driver.execs().is_empty(), "{bad}: the cell was touched");
    }
}

#[tokio::test]
async fn a_client_gets_the_rfb_banner_through_the_stream_and_the_port_is_not_forwarded() {
    let driver = Arc::new(FakeDriver::default());
    let view = open(&driver).await;
    let (mut sock, got) = banner(view.address()).await;
    assert_eq!(got, "RFB 003.008\n");
    sock.write_all(b"hello").await.unwrap();
    let mut echo = [0u8; 5];
    sock.read_exact(&mut echo).await.unwrap();
    assert_eq!(&echo, b"hello");
    // The only way to the guest's VNC port is socat over the stream.
    assert_eq!(driver.stream_argvs(), vec![vec!["socat".to_owned(), "-".to_owned(), "TCP:127.0.0.1:5901".to_owned()]]);
    view.close().await.unwrap();
}

#[tokio::test]
async fn two_clients_each_get_their_own_stream() {
    let driver = Arc::new(FakeDriver::default());
    let view = open(&driver).await;
    let (mut a, _) = banner(view.address()).await;
    let (mut b, _) = banner(view.address()).await;
    a.write_all(b"aaa").await.unwrap();
    b.write_all(b"bbb").await.unwrap();
    let (mut ea, mut eb) = ([0u8; 3], [0u8; 3]);
    a.read_exact(&mut ea).await.unwrap();
    b.read_exact(&mut eb).await.unwrap();
    assert_eq!((&ea, &eb), (b"aaa", b"bbb"));
    assert_eq!(driver.stream_argvs().len(), 2);
    view.close().await.unwrap();
}

#[tokio::test]
async fn the_plans_run_in_order_and_the_secret_is_only_ever_stdin() {
    let driver = Arc::new(FakeDriver::default());
    let mut view = open(&driver).await;
    let secret = view.secret().unwrap();
    assert_eq!(secret.len(), 8);
    assert!(secret.chars().all(|c| c.is_ascii_alphanumeric()));
    assert!(view.secret().is_none(), "the secret is given out once");

    let opened = driver.execs();
    let last = opened.last().unwrap();
    assert!(last.argv.join(" ").contains("systemctl start eidolon-view"), "{:?}", last.argv);
    let carriers: Vec<&Exec> = opened.iter().filter(|e| e.stdin.as_deref() == Some(secret.as_str())).collect();
    assert_eq!(carriers.len(), 1, "exactly one command gets the secret, as stdin");
    assert!(carriers[0].argv.iter().any(|a| a.starts_with("of=")));

    let (mut sock, _) = banner(view.address()).await;
    sock.write_all(b"x").await.unwrap();

    let url = view.url();
    let shown = format!("{view:?}");
    view.close().await.unwrap();

    let all = driver.execs();
    assert!(all.len() > opened.len());
    assert!(all.last().unwrap().argv.join(" ").contains("systemctl start eidolon-browser"));
    for argv in all.iter().map(|e| &e.argv).chain(driver.stream_argvs().iter()) {
        assert!(argv.iter().all(|a| !a.contains(&secret)), "secret in argv {argv:?}");
    }
    assert!(!shown.contains(&secret) && !url.contains(&secret));
    assert!(!format!("{:?}", all.first().map(|e| e.argv.clone())).contains(&secret));
}

#[tokio::test]
async fn closing_stops_the_listener_and_drops_connections() {
    let driver = Arc::new(FakeDriver::default());
    let view = open(&driver).await;
    let addr = view.address();
    let (mut sock, _) = banner(addr).await;
    view.close().await.unwrap();
    let mut rest = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), sock.read_to_end(&mut rest)).await.unwrap();
    assert!(read.is_err() || rest.is_empty(), "the connection should be closed");
    assert!(TcpStream::connect(addr).await.is_err(), "nothing listens after close");
}

#[tokio::test]
async fn dropping_a_view_starts_the_browser_again() {
    let driver = Arc::new(FakeDriver::default());
    let view = open(&driver).await;
    drop(view);
    for _ in 0..100 {
        if driver.execs().last().is_some_and(|e| e.argv.join(" ").contains("systemctl start eidolon-browser")) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the browser plan did not run after the drop");
}

#[tokio::test]
async fn the_marker_appears_while_the_window_is_open_and_goes_when_it_closes() {
    let scratch = Scratch::new();
    let guard = ViewGuard::new(&scratch.0);
    assert!(!guard.is_open());
    assert!(guard.refuse_if_open().is_ok());

    let driver = Arc::new(FakeDriver::default());
    let options = OpenOptions { guard: Some(guard.clone()), ..OpenOptions::default() };
    let view = View::open_with(driver, cell(), ports(), options).await.unwrap();
    assert!(guard.is_open());
    assert!(guard.marker().ends_with("view.open"));
    let err = guard.refuse_if_open().unwrap_err();
    assert_eq!(err.to_string(), "The sign-in window is open; close it to drive the browser.");

    view.close().await.unwrap();
    assert!(!guard.is_open());
    assert!(!guard.marker().exists());
}

#[tokio::test]
async fn a_marker_whose_process_is_gone_counts_as_closed() {
    let scratch = Scratch::new();
    let guard = ViewGuard::new(&scratch.0);
    for pid in ["0", "4294967295", "not a pid", ""] {
        std::fs::write(guard.marker(), format!("{pid}\n")).unwrap();
        assert!(!guard.is_open(), "pid {pid:?}");
        assert!(guard.refuse_if_open().is_ok());
    }
}
