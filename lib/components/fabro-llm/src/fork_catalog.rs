//! The denkhaus fork's catalog overlay — fork-owned file (merge-upstream
//! seam pattern, user directive 2026-09-19).
//!
//! The overlay data lives in `fork-catalog-overlay.toml` next to this
//! module; `catalog.rs` layers it with one `.toml_layer()` line in
//! `build_catalog`, between the lithos built-ins and the operator's
//! `[llm]` overlay (later layers win, so operator settings still
//! override it). Ported from the fork's `fabro-model` provider TOMLs
//! when upstream deleted that crate for the lithos catalog; carries the
//! zai glm-5.3 default and the glm-4.7 always-reasoning probe fix
//! (fabro-cd27).

/// The denkhaus fork's catalog overlay, layered between the lithos
/// built-ins and the operator's `[llm]` overlay.
pub(crate) const OVERLAY: &str = include_str!("fork-catalog-overlay.toml");

#[cfg(test)]
mod tests {
    use fabro_config::LlmLayer;

    use crate::catalog::build_catalog;

    /// Presence pin (two-pin rule, merge-upstream touchpoints): proves the
    /// one-line seam in `catalog::build_catalog` still layers this overlay.
    /// Without the seam the zai default reverts to the lithos built-in and
    /// the glm-4.7 always-reasoning fix (fabro-cd27) is gone.
    #[test]
    fn fork_catalog_overlay_is_layered_by_build_catalog() {
        let catalog = build_catalog(&LlmLayer::default(), &|_| None)
            .expect("builtin + fork overlay catalog builds");
        let zai = catalog.provider("zai").expect("zai provider exists");
        assert_eq!(
            zai.default_model(),
            Some("glm-5.3"),
            "the fork overlay pins the zai default"
        );
        let glm47 = catalog
            .model("zai", "glm-4.7")
            .expect("zai glm-4.7 model exists");
        assert!(
            glm47.capabilities().reasoning().is_supported(),
            "the fork overlay marks glm-4.7 as always-reasoning (fabro-cd27)"
        );
    }
}
