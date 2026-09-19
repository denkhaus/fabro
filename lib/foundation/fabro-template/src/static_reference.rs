//! Static file references in workflow packages.
//!
//! A workflow names other files from its graph (`import`,
//! `stack.child_workflow`, `@`-prefixed `prompt`, `output_schema` and `goal`
//! values) and from its `workflow.toml` (a Dockerfile, a run goal file).
//! These references are *static*: they may not contain template syntax,
//! because they are resolved before template rendering. The graph walker that
//! finds them lives in `fabro-dot`; this module owns the one rule every
//! consumer applies to a reference before resolving it.

use fabro_types::ReferenceKind;

use crate::contains_template_syntax;

/// A static file reference that unexpectedly contains template syntax.
#[derive(Debug, thiserror::Error)]
#[error("templates are not supported in {kind}s: {value}")]
pub struct StaticReferenceError {
    kind:  ReferenceKind,
    value: String,
}

impl StaticReferenceError {
    #[must_use]
    pub fn new(kind: ReferenceKind, value: impl Into<String>) -> Self {
        Self {
            kind,
            value: value.into(),
        }
    }

    #[must_use]
    pub fn kind(&self) -> ReferenceKind {
        self.kind
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Reject static file references (imports, child workflows, `@` file values)
/// that contain template syntax.
pub fn validate_static_reference(
    value: &str,
    kind: ReferenceKind,
) -> Result<(), StaticReferenceError> {
    if contains_template_syntax(value) {
        return Err(StaticReferenceError::new(kind, value));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use fabro_types::ReferenceKind;

    use super::validate_static_reference;

    #[test]
    fn static_reference_rejects_template_syntax() {
        let error = validate_static_reference(
            "@schemas/{{ inputs.schema }}.json",
            ReferenceKind::FileInline,
        )
        .unwrap_err();

        assert_eq!(error.kind(), ReferenceKind::FileInline);
        assert_eq!(error.value(), "@schemas/{{ inputs.schema }}.json");
        assert!(
            error
                .to_string()
                .contains("templates are not supported in file inline references"),
            "unexpected error: {error}",
        );
        assert!(
            validate_static_reference("@schemas/result.json", ReferenceKind::FileInline).is_ok()
        );
    }
}
