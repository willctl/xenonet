//! fetch(), as the binary has it: HTTPS with rustls on ring and the
//! system's own certificates, through the system's proxy, one client for
//! the whole service. What the cores asked fetch for during the port.

use std::sync::OnceLock;
use std::time::Duration;

pub struct Fetched {
    pub status: u16,
    /// Names lowercased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Fetched {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// The service's one client.
pub fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
        reqwest::Client::builder().user_agent(concat!("xenonet/", env!("CARGO_PKG_VERSION"))).build().expect("an HTTPS client")
    })
}

/// One request, within `timeout`: its answer, or why there was none, in
/// the words fetch's errors have ("fetch failed: …", "The operation was
/// aborted due to timeout").
pub async fn fetch(method: &str, url: &str, headers: &[(String, String)], body: Option<Vec<u8>>, timeout: Duration) -> Result<Fetched, String> {
    let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| format!("'{method}' is not a valid HTTP method."))?;
    let mut req = client().request(method, url).timeout(timeout);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if let Some(b) = body {
        req = req.body(b);
    }
    let res = req.send().await.map_err(|e| if e.is_timeout() { "The operation was aborted due to timeout".to_string() } else { format!("fetch failed: {e}") })?;
    let status = res.status().as_u16();
    let headers = res.headers().iter().map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect();
    let body = res.bytes().await.map_err(|e| if e.is_timeout() { "The operation was aborted due to timeout".to_string() } else { format!("fetch failed: {e}") })?.to_vec();
    Ok(Fetched { status, headers, body })
}
