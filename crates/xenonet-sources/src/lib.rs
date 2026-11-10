//! What XenoNet reads: the transcripts' lines (`lines`) and their events,
//! the records the adapters' readers make (`record`, `session`), the
//! native readers' contract and numbat's, what the transcripts say as
//! they're written (`live`), Codex's archived rollouts (`archive`), the
//! shared context (`xenolith`, `wiki`), plan usage and refusals (with the
//! service's Refusals core, `refusals::runtime`).

pub mod archive;
pub mod claude_usage;
pub mod desktop_connectors;
pub mod git;
pub mod line_events;
pub mod lines;
pub mod live;
pub mod native;
pub mod numbat;
pub mod openrouter_usage;
pub mod record;
pub mod refusals;
pub mod reported_runs;
pub mod session;
pub mod wiki;
pub mod xenolith;
