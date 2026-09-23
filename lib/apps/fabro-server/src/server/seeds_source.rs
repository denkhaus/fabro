//! The seeds read API's data source (fabro-3488, ADR-0023 step 5).
//!
//! The read-only seeds endpoints (`/api/v1/seeds*`) serve tracker state
//! through one seam: a [`SeedsSource`] produces a [`SeedsSnapshot`] — a
//! parsed [`seeds::Store`] plus the commit it was read at. The production
//! source is configured at startup; an unconfigured server serves the
//! documented `503` instead of guessing a checkout. The snapshot type is
//! cheap to share (`Arc`) so every request of a refresh window reads the
//! same parsed store without re-parsing the JSONL files.

use std::fmt;
use std::sync::Arc;

use seeds::Store;

/// A parsed tracker state plus where it came from.
pub struct SeedsSnapshot {
    /// The parsed `.seeds/` store.
    pub store:  Arc<Store>,
    /// The commit the snapshot was taken at, when the source knows it.
    // Transitional (fabro-3488): the production source under the pending
    // fork decision fills this; the fixture pins its handoff meanwhile.
    #[allow(
        dead_code,
        reason = "the production source under the pending fork decision fills this"
    )]
    pub commit: Option<String>,
}

/// Why a seeds snapshot could not be served.
#[derive(Debug)]
pub enum SeedsSourceError {
    /// No seeds source is configured; the endpoints answer `503`.
    Unconfigured,
    /// The configured source failed to refresh or read.
    // Transitional (fabro-3488): constructed by the production source
    // under the pending fork decision; the failing-source 503 test pins
    // the mapping meanwhile.
    #[allow(
        dead_code,
        reason = "the production source under the pending fork decision constructs this"
    )]
    Unavailable(String),
}

impl fmt::Display for SeedsSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unconfigured => {
                f.write_str("no seeds source is configured (server.seeds settings)")
            }
            Self::Unavailable(reason) => write!(f, "seeds source unavailable: {reason}"),
        }
    }
}

/// The read-side seam the seeds handlers serve from.
#[async_trait::async_trait]
pub trait SeedsSource: Send + Sync {
    /// Return a fresh-enough snapshot of the tracker store.
    ///
    /// Implementations decide freshness (refresh window); callers must not
    /// assume a snapshot reflects writes newer than the last refresh.
    async fn snapshot(&self) -> Result<SeedsSnapshot, SeedsSourceError>;
}

/// The source of a server without seeds configuration: every read is the
/// documented `503`, never a guessed checkout.
pub(crate) struct DisabledSeedsSource;

#[async_trait::async_trait]
impl SeedsSource for DisabledSeedsSource {
    async fn snapshot(&self) -> Result<SeedsSnapshot, SeedsSourceError> {
        Err(SeedsSourceError::Unconfigured)
    }
}
