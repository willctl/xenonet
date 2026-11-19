use crate::error::{Result, ViewError};
use crate::guard::ViewGuard;
use crate::secret;
use adytum_driver::Driver;
use adytum_model::CellName;
use eidolon_browser::units::Ports;
use eidolon_browser::{start_browser_plan, start_view_plan, stream_argv};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::task::{JoinHandle, JoinSet};

/// How a window is opened beyond the defaults.
#[derive(Debug, Clone)]
pub struct OpenOptions {
    /// Where to listen. Only 127.0.0.1 is accepted; anything else is refused
    /// before the cell is touched. The field exists so that refusal can be
    /// tested.
    pub bind: IpAddr,
    /// The port to listen on; 0 lets the system pick.
    pub port: u16,
    /// The marker to write while the window is open.
    pub guard: Option<ViewGuard>,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self { bind: IpAddr::V4(Ipv4Addr::LOCALHOST), port: 0, guard: None }
    }
}

struct Restore {
    driver: Arc<dyn Driver>,
    cell: CellName,
    guard: Option<ViewGuard>,
}

/// An open sign-in window: a listener on 127.0.0.1 that reaches the guest's
/// VNC port through the driver's stream.
pub struct View {
    addr: SocketAddr,
    secret: Option<String>,
    task: Option<JoinHandle<()>>,
    restore: Option<Restore>,
}

// The secret is never printed.
impl fmt::Debug for View {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("View").field("address", &self.addr).finish_non_exhaustive()
    }
}

impl View {
    /// Opens the window with the defaults (a port the system picks, no marker).
    pub async fn open(driver: Arc<dyn Driver>, cell: CellName, ports: Ports) -> Result<View> {
        View::open_with(driver, cell, ports, OpenOptions::default()).await
    }

    /// Opens the window: listens first, then switches the cell to the view
    /// units (which stops automation) with a fresh secret.
    pub async fn open_with(driver: Arc<dyn Driver>, cell: CellName, ports: Ports, options: OpenOptions) -> Result<View> {
        if options.bind != IpAddr::V4(Ipv4Addr::LOCALHOST) {
            return Err(ViewError::NotLoopback { address: options.bind });
        }
        let listener = TcpListener::bind(SocketAddr::new(options.bind, options.port)).await.map_err(|source| ViewError::Io { what: "listen for the sign-in window", source })?;
        let addr = listener.local_addr().map_err(|source| ViewError::Io { what: "read the window's address", source })?;
        let secret = secret::make()?;

        // Mark first, so automation is refused before the browser is stopped.
        if let Some(guard) = &options.guard {
            guard.mark_open()?;
        }
        if let Err(e) = start_view_plan(&secret).run(driver.as_ref(), &cell).await {
            // Put back what may be half switched.
            let _ = start_browser_plan().run(driver.as_ref(), &cell).await;
            if let Some(guard) = &options.guard {
                let _ = guard.clear();
            }
            return Err(e.into());
        }

        let task = tokio::spawn(accept_loop(listener, Arc::clone(&driver), cell.clone(), ports.vnc));
        Ok(View { addr, secret: Some(secret), task: Some(task), restore: Some(Restore { driver, cell, guard: options.guard }) })
    }

    /// `127.0.0.1:<port>`.
    pub fn address(&self) -> SocketAddr {
        self.addr
    }

    /// `vnc://127.0.0.1:<port>`, without the secret.
    pub fn url(&self) -> String {
        format!("vnc://{}", self.addr)
    }

    /// The screen share's password for this session. It is handed out once,
    /// for the person to type into a VNC viewer; a second call gives `None`.
    pub fn secret(&mut self) -> Option<String> {
        self.secret.take()
    }

    /// Stops listening, drops every connection, and starts automation again.
    pub async fn close(mut self) -> Result<()> {
        if let Some(task) = self.task.take() {
            task.abort();
            // Waiting makes sure the listener and the connections are gone.
            let _ = task.await;
        }
        let Some(restore) = self.restore.take() else { return Ok(()) };
        if let Some(guard) = &restore.guard {
            guard.clear()?;
        }
        start_browser_plan().run(restore.driver.as_ref(), &restore.cell).await?;
        Ok(())
    }
}

impl Drop for View {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        let Some(restore) = self.restore.take() else { return };
        if let Some(guard) = &restore.guard {
            if let Err(e) = guard.clear() {
                eprintln!("eidolon-view: {e}");
            }
        }
        // Dropping cannot wait, so the plan runs in the background if a
        // runtime is there to run it.
        match Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    if let Err(e) = start_browser_plan().run(restore.driver.as_ref(), &restore.cell).await {
                        eprintln!("eidolon-view: could not start the browser again: {e}");
                    }
                });
            }
            Err(_) => eprintln!("eidolon-view: the window was dropped outside a runtime, so the browser was not started again; run `eidolon view close`."),
        }
    }
}

async fn accept_loop(listener: TcpListener, driver: Arc<dyn Driver>, cell: CellName, vnc_port: u16) {
    // Dropping the set (when this task is aborted) aborts every connection.
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((socket, _)) => {
                    connections.spawn(serve(socket, Arc::clone(&driver), cell.clone(), vnc_port));
                }
                Err(e) => {
                    eprintln!("eidolon-view: could not accept a connection: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}

async fn serve(mut socket: TcpStream, driver: Arc<dyn Driver>, cell: CellName, vnc_port: u16) {
    let mut stream = match driver.stream(&cell, &stream_argv(vnc_port)).await {
        Ok(stream) => stream,
        Err(e) => {
            eprintln!("eidolon-view: could not reach the window in the cell: {e}");
            return;
        }
    };
    // A viewer that disconnects ends the copy; nothing more to do then.
    let _ = copy_bidirectional(&mut socket, &mut stream).await;
}
