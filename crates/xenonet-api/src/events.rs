//! This machine's event stream, as the page reads it (GET /api/events),
//! for what follows it in-process: the relay that serves this host's map
//! to a console on another host. The service keeps the stream itself.

pub use bytes::Bytes;
use tokio::sync::mpsc;

pub trait EventStream: Send + Sync {
    /// A reader's stream of frames (`event: <name>\ndata: <JSON>\n\n`),
    /// with this machine's own view of each event when `own_only`. It ends
    /// when the receiver is dropped.
    fn subscribe(&self, own_only: bool) -> mpsc::UnboundedReceiver<Bytes>;
}
