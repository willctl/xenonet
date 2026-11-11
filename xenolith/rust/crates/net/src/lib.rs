//! Xenolith between machines, on XenoNet's transport library (xenonet-net): its own iroh endpoint,
//! the sync protocol and joining. Empty so far. It runs in xenolith-service alone, on a tokio
//! runtime of its own, so the hook, the stdio server and the tray (xenolith-core only) never start
//! or link it.
