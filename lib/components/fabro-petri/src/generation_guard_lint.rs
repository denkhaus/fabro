//! The generation-guard-vs-breaker lint at create (fabro-51ad): a
//! `nodes.<id>.generation >= N` exit guard needs N+1 completed visits of
//! the guarded node, while the engine's failure circuit breaker
//! (`loop_restart_signature_limit`, Fabro default 3) can cut the loop's
//! continuation at its limit first. The breaker (post option a) passes a
//! route that LEAVES the cycle, but a back-edge continuation between the
//! limit and the guard's threshold still dies as `workflow_error`. This
//! lint names the collision at admission, with the recommendation, before
//! a run ever hits it.

use crate::fork_stage_envelope::LintSeverity;

/// One generation-guard finding over the raw DOT `graph_source`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationGuardLint {
    pub severity: LintSeverity,
    pub message:  String,
}

/// Compare every `nodes.<id>.generation` exit condition in the graph text
/// against `signature_limit` (the run policy's breaker limit, when one is
/// set). A guard `>= N` fires after N+1 completions; when that is beyond
/// the limit, the breaker can cut the cycle before the guard's exit —
/// warn with the recommendation.
#[must_use]
pub fn lint(graph_source: &str, signature_limit: Option<u32>) -> Vec<GenerationGuardLint> {
    let Some(limit) = signature_limit else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    for guard in generation_guards(graph_source) {
        // `>= N` fires after N+1 completions; `> M` after M+1.
        let fires_after = guard.threshold + 1;
        if fires_after > limit {
            let threshold = guard.threshold;
            findings.push(GenerationGuardLint {
                severity: LintSeverity::Warning,
                message:  format!(
                    "edge '{edge}' guards on nodes.{node}.generation >= {threshold} — the guard \
                     fires after {fires_after} completions, beyond the circuit breaker's \
                     loop_restart_signature_limit={limit}, which can cut the loop's continuation \
                     first and end the run workflow_error instead of at the guarded exit. \
                     Recommendation: set loop_restart_signature_limit above {threshold} (or \
                     lower the guard threshold below {limit}).",
                    edge = guard.edge,
                    node = guard.node
                ),
            });
        }
    }
    findings
}

/// One `nodes.<id>.generation <op> <number>` clause on an edge condition.
struct Guard {
    edge:      String,
    node:      String,
    threshold: u32,
}

/// The generation guards in the DOT text: edges whose `condition=`
/// attribute compares `nodes.<id>.generation` with a number.
fn generation_guards(graph_source: &str) -> Vec<Guard> {
    let mut guards = Vec::new();
    let mut rest = graph_source;
    while let Some(open) = rest.find('[') {
        let head = &rest[..open];
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else { break };
        let block = &after[..close];
        rest = &after[close + 1..];
        let Some(condition) = attribute(block, "condition") else {
            continue;
        };
        let subject = head
            .lines()
            .last()
            .unwrap_or_default()
            .trim()
            .trim_matches(|c: char| c.is_whitespace() || c == ';' || c == ',');
        if !subject.contains("->") {
            continue;
        }
        guards.extend(guards_in_condition(subject, condition));
    }
    guards
}

/// The `nodes.<id>.generation >= N` (or `> M`) clauses of one condition,
/// attributed to `edge`.
fn guards_in_condition(edge: &str, condition: &str) -> Vec<Guard> {
    let mut guards = Vec::new();
    let mut rest = condition;
    while let Some(at) = rest.find("nodes.") {
        let after = &rest[at + "nodes.".len()..];
        let Some((node, tail)) = after.split_once('.') else {
            break;
        };
        let tail = tail.trim_start_matches("generation");
        let tail = tail.trim_start();
        let (threshold, strict) = if let Some(number) = tail.strip_prefix(">=") {
            (number, false)
        } else if let Some(number) = tail.strip_prefix('>') {
            (number, true)
        } else {
            break;
        };
        // `> M` is `>= M+1`.
        let parsed = threshold.trim().parse::<u32>().ok().map(|value| {
            if strict {
                value.saturating_add(1)
            } else {
                value
            }
        });
        if let Some(threshold) = parsed {
            if !node.is_empty() {
                guards.push(Guard {
                    edge: edge.to_string(),
                    node: node.to_string(),
                    threshold,
                });
            }
        }
        let Some(next) = after.find("generation") else {
            break;
        };
        rest = &after[next..];
    }
    guards
}

/// The quoted-or-bare value of `name=` in an attribute block.
fn attribute<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=");
    let at = block.find(&key)?;
    let rest = &block[at + key.len()..];
    let trimmed = rest.trim_start();
    if let Some(value) = trimmed.strip_prefix('"') {
        return value.find('"').map(|end| &value[..end]);
    }
    let end = trimmed
        .find([',', ']', '\n', ' ', '\r'])
        .unwrap_or(trimmed.len());
    Some(&trimmed[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUARDED: &str = r#"digraph W {
        graph [goal="G"]
        start [shape=Mdiamond]
        exit  [shape=Msquare]
        flaky [script="false"]
        start -> flaky
        flaky -> exit [x.kind="deadlock", condition="nodes.flaky.generation >= 2"]
        flaky -> retry
        retry -> flaky
    }"#;

    #[test]
    fn a_guard_below_the_limit_is_silent() {
        assert!(lint(GUARDED, Some(3)).is_empty(), "fires after 3, limit 3");
    }

    #[test]
    fn a_guard_beyond_the_limit_warns_with_the_recommendation() {
        let findings = lint(GUARDED, Some(2));
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("nodes.flaky.generation"));
        assert!(findings[0].message.contains("loop_restart_signature_limit"));
        assert!(findings[0].message.contains("Recommendation"));
    }

    #[test]
    fn a_greater_than_guard_counts_one_higher() {
        let source = r#"a -> exit [condition="nodes.a.generation > 2"]"#;
        assert_eq!(lint(source, Some(3)).len(), 1, "> 2 fires after 4 > 3");
        assert!(lint(source, Some(4)).is_empty(), "> 2 fires after 4 <= 4");
    }

    #[test]
    fn no_breaker_no_warning() {
        assert!(lint(GUARDED, None).is_empty());
    }

    #[test]
    fn multiple_guards_each_lint() {
        let source = "a -> e1 [condition=\"nodes.a.generation >= 5\"]\nb -> e2 [condition=\"nodes.b.generation >= 5\"]";
        assert_eq!(lint(source, Some(3)).len(), 2);
    }
}
