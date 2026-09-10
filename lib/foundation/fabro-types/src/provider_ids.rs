//! Well-known provider identifiers.
//!
//! Provider identity is open-ended catalog data, so [`ProviderId`] is a plain
//! string newtype. The first-party providers are named here because code
//! paths such as Codex login and the install flow refer to them directly.

use lithos_llm::catalog::ProviderId;

pub const ANTHROPIC: &str = "anthropic";
pub const OPENAI: &str = "openai";
/// The ChatGPT-subscription deployment that stands in for [`OPENAI`] when a
/// Codex OAuth credential is present.
pub const OPENAI_CODEX: &str = "openai-codex";
pub const GEMINI: &str = "gemini";

#[must_use]
pub fn anthropic() -> ProviderId {
    ProviderId::new(ANTHROPIC)
}

#[must_use]
pub fn openai() -> ProviderId {
    ProviderId::new(OPENAI)
}

#[must_use]
pub fn openai_codex() -> ProviderId {
    ProviderId::new(OPENAI_CODEX)
}

#[must_use]
pub fn gemini() -> ProviderId {
    ProviderId::new(GEMINI)
}
