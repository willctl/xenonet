//! Registering actors (core/actors/registration.js;
//! docs/design/actors/interface.md, section 2).

/// A token unused for 30 days stops working.
pub const TOKEN_UNUSED_MS: f64 = 30.0 * 24.0 * 3600e3;

/// Whether a token last used (or made) `at` has gone unused for 30 days by
/// `t`: from that very millisecond, wherever it's asked.
pub fn token_expired(at: Option<f64>, t: f64) -> bool {
    at.is_none_or(|at| t - at >= TOKEN_UNUSED_MS)
}
