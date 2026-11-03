//! What XenoNet reads: the transcripts' lines (`lines`), the readers built on
//! them (Claude Code's, Codex's, the native readers', numbat's) and the
//! sessions each writes (`session`), what the transcripts say as they're
//! written (`live`), the shared context (`xenolith`, `wiki`), plan usage and
//! refusals.

pub mod claude;
pub mod claude_usage;
pub mod codex;
pub mod desktop_connectors;
pub mod git;
pub mod grok;
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
