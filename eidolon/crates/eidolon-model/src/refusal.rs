use serde::{Deserialize, Serialize};
use std::fmt;

/// Why a rule said no. The codes are stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalCode {
    NotAHolder,
    SecondSeat,
    StaleGeneration,
    SeatNotSignedByHolder,
    WrongWall,
    ChangedChunk,
    LeaveOutEscapes,
    Retired,
}

impl RefusalCode {
    pub fn as_str(self) -> &'static str {
        match self {
            RefusalCode::NotAHolder => "not_a_holder",
            RefusalCode::SecondSeat => "second_seat",
            RefusalCode::StaleGeneration => "stale_generation",
            RefusalCode::SeatNotSignedByHolder => "seat_not_signed_by_holder",
            RefusalCode::WrongWall => "wrong_wall",
            RefusalCode::ChangedChunk => "changed_chunk",
            RefusalCode::LeaveOutEscapes => "leave_out_escapes",
            RefusalCode::Retired => "retired",
        }
    }
}

/// A rule's no: a code to match on, what happened, and what to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub code: RefusalCode,
    pub message: String,
    pub fix: String,
}

impl Refusal {
    pub fn new(code: RefusalCode, message: impl Into<String>, fix: impl Into<String>) -> Refusal {
        Refusal { code, message: message.into(), fix: fix.into() }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "eidolon: {} {}", self.message, self.fix)
    }
}

impl std::error::Error for Refusal {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_snake_case_and_display_is_message_then_fix() {
        let all = [
            (RefusalCode::NotAHolder, "not_a_holder"),
            (RefusalCode::SecondSeat, "second_seat"),
            (RefusalCode::StaleGeneration, "stale_generation"),
            (RefusalCode::SeatNotSignedByHolder, "seat_not_signed_by_holder"),
            (RefusalCode::WrongWall, "wrong_wall"),
            (RefusalCode::ChangedChunk, "changed_chunk"),
            (RefusalCode::LeaveOutEscapes, "leave_out_escapes"),
            (RefusalCode::Retired, "retired"),
        ];
        for (code, text) in all {
            assert_eq!(code.as_str(), text);
            assert_eq!(serde_json::to_string(&code).unwrap_or_default(), format!("\"{text}\""));
        }
        let r = Refusal::new(RefusalCode::Retired, "That eidolon is retired.", "Make a new one.");
        assert_eq!(r.to_string(), "eidolon: That eidolon is retired. Make a new one.");
    }
}
