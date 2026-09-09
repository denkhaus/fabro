//! Helpers over the lithos request-control enums.
//!
//! lithos owns [`ReasoningEffort`] and [`Speed`] and marks both
//! `#[non_exhaustive]`. Fabro needs to list, name, and parse them for
//! settings, graph attributes, and CLI flags, so the spellings live here in
//! one place. The names match the lithos serde form.

pub use lithos_llm::types::{ReasoningEffort, Speed};

/// Every reasoning effort, least to most.
pub const REASONING_EFFORTS: &[ReasoningEffort] = &[
    ReasoningEffort::Minimal,
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
    ReasoningEffort::Xhigh,
    ReasoningEffort::Max,
];

/// Every speed tier.
pub const SPEEDS: &[Speed] = &[Speed::Fast, Speed::Balanced, Speed::Economical];

/// The wire spelling of a reasoning effort.
#[must_use]
pub fn reasoning_effort_name(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::Minimal => "minimal",
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
        ReasoningEffort::Xhigh => "xhigh",
        ReasoningEffort::Max => "max",
        _ => "unknown",
    }
}

/// The wire spelling of a speed tier.
#[must_use]
pub fn speed_name(speed: Speed) -> &'static str {
    match speed {
        Speed::Fast => "fast",
        Speed::Balanced => "balanced",
        Speed::Economical => "economical",
        _ => "unknown",
    }
}

/// Parses a reasoning effort from its wire spelling.
#[must_use]
pub fn parse_reasoning_effort(value: &str) -> Option<ReasoningEffort> {
    REASONING_EFFORTS
        .iter()
        .copied()
        .find(|effort| reasoning_effort_name(*effort) == value)
}

/// Parses a speed tier from its wire spelling.
#[must_use]
pub fn parse_speed(value: &str) -> Option<Speed> {
    SPEEDS
        .iter()
        .copied()
        .find(|speed| speed_name(*speed) == value)
}

/// Position of an effort in the least-to-most ordering.
fn effort_rank(effort: ReasoningEffort) -> usize {
    REASONING_EFFORTS
        .iter()
        .position(|candidate| *candidate == effort)
        .unwrap_or(REASONING_EFFORTS.len())
}

/// Selects the supported effort nearest to `requested`.
///
/// When two supported values are equally distant, the higher effort wins.
/// Returns `None` when nothing is supported.
#[must_use]
pub fn closest_supported_effort(
    requested: ReasoningEffort,
    supported: impl Fn(ReasoningEffort) -> bool,
) -> Option<ReasoningEffort> {
    let target = effort_rank(requested);
    REASONING_EFFORTS
        .iter()
        .copied()
        .filter(|effort| supported(*effort))
        .min_by_key(|effort| {
            let rank = effort_rank(*effort);
            (rank.abs_diff(target), std::cmp::Reverse(rank))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_through_serde() {
        for effort in REASONING_EFFORTS {
            let json = serde_json::to_string(effort).unwrap();
            assert_eq!(json, format!("\"{}\"", reasoning_effort_name(*effort)));
            assert_eq!(
                parse_reasoning_effort(reasoning_effort_name(*effort)),
                Some(*effort)
            );
        }
        for speed in SPEEDS {
            let json = serde_json::to_string(speed).unwrap();
            assert_eq!(json, format!("\"{}\"", speed_name(*speed)));
            assert_eq!(parse_speed(speed_name(*speed)), Some(*speed));
        }
        assert_eq!(parse_reasoning_effort("standard"), None);
        assert_eq!(parse_speed("standard"), None);
    }

    #[test]
    fn closest_supported_prefers_the_higher_neighbor_on_ties() {
        let supported = |effort| matches!(effort, ReasoningEffort::Low | ReasoningEffort::High);
        assert_eq!(
            closest_supported_effort(ReasoningEffort::Medium, supported),
            Some(ReasoningEffort::High)
        );
        assert_eq!(
            closest_supported_effort(ReasoningEffort::Max, supported),
            Some(ReasoningEffort::High)
        );
        assert_eq!(
            closest_supported_effort(ReasoningEffort::Minimal, supported),
            Some(ReasoningEffort::Low)
        );
        assert_eq!(
            closest_supported_effort(ReasoningEffort::Medium, |_| false),
            None
        );
    }
}
