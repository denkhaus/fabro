//! Helpers over the lithos request-control enums.
//!
//! lithos owns [`ReasoningEffort`] and [`Speed`], their spellings, and their
//! parsing (`ALL`, `as_str`, `Display`, `FromStr`). What stays here is
//! Fabro's own rule for substituting a reasoning level a model lacks.

pub use lithos_llm::types::{ReasoningEffort, Speed};

/// Position of an effort in the least-to-most ordering.
fn effort_rank(effort: ReasoningEffort) -> usize {
    ReasoningEffort::ALL
        .iter()
        .position(|candidate| *candidate == effort)
        .unwrap_or(ReasoningEffort::ALL.len())
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
    ReasoningEffort::ALL
        .into_iter()
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
    fn lithos_spellings_match_serde() {
        for effort in ReasoningEffort::ALL {
            let json = serde_json::to_string(&effort).unwrap();
            assert_eq!(json, format!("\"{effort}\""));
            assert_eq!(effort.as_str().parse::<ReasoningEffort>().unwrap(), effort);
        }
        for speed in Speed::ALL {
            let json = serde_json::to_string(&speed).unwrap();
            assert_eq!(json, format!("\"{speed}\""));
            assert_eq!(speed.as_str().parse::<Speed>().unwrap(), speed);
        }
        assert!("standard".parse::<ReasoningEffort>().is_err());
        assert!("standard".parse::<Speed>().is_err());
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
