//! Captured workspace files go to the server's configured artifact store.
//! Engine values and patches keep using the separate blob capability.

use bytes::Bytes;
use fabro_client::Client;
use fabro_store::ArtifactStore;
use fabro_types::{BlobHash, RunId};

#[derive(Debug, thiserror::Error)]
pub enum ArtifactWriteError {
    #[error("artifact storage failed")]
    Store(#[from] fabro_store::Error),
    #[error("artifact upload failed")]
    Upload(#[source] anyhow::Error),
}

/// Storage for captured files, keyed by the run the hooks record them under
/// and the `digest` of `bytes`. Implementations reject bytes that do not
/// match `digest`, publish complete objects before returning, preserve
/// errors, and permit concurrent and repeated writes of identical content.
#[async_trait::async_trait]
pub trait ArtifactWriter: Send + Sync {
    async fn write(
        &self,
        run_id: &RunId,
        digest: &BlobHash,
        bytes: Bytes,
    ) -> Result<(), ArtifactWriteError>;
}

pub struct StoreArtifactWriter {
    store: ArtifactStore,
}

impl StoreArtifactWriter {
    #[must_use]
    pub fn new(store: ArtifactStore) -> Self {
        Self { store }
    }
}

#[async_trait::async_trait]
impl ArtifactWriter for StoreArtifactWriter {
    async fn write(
        &self,
        run_id: &RunId,
        digest: &BlobHash,
        bytes: Bytes,
    ) -> Result<(), ArtifactWriteError> {
        self.store
            .put_capture(run_id, digest, bytes)
            .await
            .map_err(ArtifactWriteError::from)
    }
}

/// Uploads to the server, which checks the digest before storing.
pub struct ClientArtifactWriter {
    client: Client,
}

impl ClientArtifactWriter {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl ArtifactWriter for ClientArtifactWriter {
    async fn write(
        &self,
        run_id: &RunId,
        digest: &BlobHash,
        bytes: Bytes,
    ) -> Result<(), ArtifactWriteError> {
        self.client
            .write_run_artifact_content(run_id, digest, bytes)
            .await
            .map_err(ArtifactWriteError::Upload)
    }
}
