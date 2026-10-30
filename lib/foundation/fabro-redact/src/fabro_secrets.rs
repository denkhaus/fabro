//! Deterministic redaction rules for Fabro's own credential names.
//!
//! The entropy layer catches high-entropy values by shape and the
//! gitleaks layer knows third-party secret formats — but neither is
//! anchored on FABRO'S OWN credential NAMES, so the deterministic
//! coverage every named secret deserves was missing (work#1077): the
//! per-run channel token sits in every sandbox env, and one `env` dump
//! in any stage would store it in a run record — re-opening the
//! principal-confusion case work#1043 closed, through the storage path
//! instead of the socket.
//!
//! Two anchored forms, both keyed off the single-source constants in
//! fabro-static so the redaction cannot drift from the wire spelling:
//! the env-assignment form (`FABRO_RUN_CHANNEL_TOKEN=<value>`) and the
//! header form (`x-fabro-run-channel-token: <value>` — how HTTP debug
//! output renders it). The NAME anchors the match, so ANY value
//! redacts — including the low-entropy test values the entropy layer
//! would let through, which is the point of deterministic name
//! coverage. A bare value with no name around it has no anchor and is
//! the entropy layer's case (64 hex chars are high-entropy by shape).

use std::sync::LazyLock;

use regex::Regex;

use super::Region;

static RUN_CHANNEL_TOKEN_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)(?:{}[=:]\s*)(\S+)",
        fabro_static::EnvVars::FABRO_RUN_CHANNEL_TOKEN
    ))
    .expect("the run-channel token assignment pattern must compile")
});

static RUN_CHANNEL_TOKEN_HEADER_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)(?:{}:\s*)(\S+)",
        fabro_static::RUN_CHANNEL_TOKEN_HEADER
    ))
    .expect("the run-channel token header pattern must compile")
});

/// Regions covering the VALUE of every fabro-named credential form in
/// `s` (the name stays visible, mirroring the gitleaks layer's
/// `key=REDACTED` rendering).
pub(crate) fn find_regions(s: &str) -> Vec<Region> {
    let mut regions = Vec::new();
    for pattern in [
        &*RUN_CHANNEL_TOKEN_ASSIGNMENT,
        &*RUN_CHANNEL_TOKEN_HEADER_VALUE,
    ] {
        for captures in pattern.captures_iter(s) {
            if let Some(value) = captures.get(1) {
                regions.push(Region {
                    start: value.start(),
                    end:   value.end(),
                });
            }
        }
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::super::redact_string;

    /// A hex value low-entropy enough that the entropy layer lets it
    /// through — only the NAME rule can catch this shape.
    const LOW_ENTROPY_TOKEN: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn the_env_assignment_form_redacts_any_value() {
        let redacted = redact_string(&format!(
            "FABRO_STAGE=capture FABRO_RUN_CHANNEL_TOKEN={LOW_ENTROPY_TOKEN} HOME=/root"
        ));
        assert_eq!(
            redacted,
            "FABRO_STAGE=capture FABRO_RUN_CHANNEL_TOKEN=REDACTED HOME=/root"
        );
    }

    #[test]
    fn the_header_form_redacts_any_value_in_both_casings() {
        for rendering in [
            format!("x-fabro-run-channel-token: {LOW_ENTROPY_TOKEN}"),
            format!("X-Fabro-Run-Channel-Token: {LOW_ENTROPY_TOKEN}"),
        ] {
            assert_eq!(
                redact_string(&rendering),
                format!(
                    "{}: REDACTED",
                    rendering
                        .split(':')
                        .next()
                        .expect("a rendering with a colon")
                ),
                "the value redacts; the name keeps its input casing"
            );
        }
    }

    #[test]
    fn a_bare_hex_value_stays_raw_until_the_value_anchored_rule_lands() {
        // OPEN RESIDUAL (work#1077 second half): a bare 64-hex value —
        // `echo $FABRO_RUN_CHANNEL_TOKEN` output — has no anchor, and
        // NEITHER the entropy layer NOR a sane blanket rule covers it
        // (hex is below the entropy threshold; a blanket 64-hex rule
        // would also redact every sha256 digest in operational logs).
        // The precise fix is value-anchored: the server knows each
        // run's token and must thread it into the redaction context.
        // This pin documents the gap deliberately — flip it when that
        // rule lands.
        let bare = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert!(!redact_string(bare).contains("REDACTED"));
    }

    #[test]
    fn similar_fabro_names_stay_visible() {
        // Control: name anchoring must not spill onto unrelated
        // FABRO_* env names.
        let line = "FABRO_STAGE=capture FABRO_SERVER=/run/fabro/channel.sock";
        assert_eq!(redact_string(line), line);
    }
}
