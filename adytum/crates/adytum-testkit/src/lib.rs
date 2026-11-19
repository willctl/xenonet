//! Fakes for Adytum's tests: a fake `limactl` and `podman` that replay
//! golden transcripts, and a recorder that writes those transcripts from a
//! real run.
//!
//! Drivers take a `&dyn Runner`, so a test hands them a [`FakeRunner`]
//! loaded from a golden file instead of starting a real tool. Record a
//! golden once with [`RecordingRunner`] around a [`SystemRunner`], then
//! replay it forever. Golden files belong to the crate whose behaviour they
//! pin (the drivers); this crate keeps only the fixtures for its own tests.
//!
//! [`SystemRunner`]: adytum_exec::SystemRunner

mod fake;
mod recording;
mod transcript;

pub use fake::FakeRunner;
pub use recording::RecordingRunner;
pub use transcript::{Call, Transcript, TranscriptError};
