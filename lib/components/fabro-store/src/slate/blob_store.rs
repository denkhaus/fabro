use std::sync::Arc;

use bytes::Bytes;
use fabro_types::RunBlobId;
use futures::StreamExt;

use crate::record::{RawBytesCodec, Record, Repository};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob(pub Bytes);

impl AsRef<[u8]> for Blob {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl From<Bytes> for Blob {
    fn from(value: Bytes) -> Self {
        Self(value)
    }
}

impl Record for Blob {
    type Id = RunBlobId;
    type Codec = RawBytesCodec;

    const PREFIX: &'static str = "blobs/sha256";

    fn id(&self) -> Self::Id {
        RunBlobId::new(&self.0)
    }
}

pub struct BlobStore {
    repo: Repository<Blob>,
}

impl std::fmt::Debug for BlobStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlobStore").finish_non_exhaustive()
    }
}

impl BlobStore {
    pub(crate) fn new(db: Arc<slatedb::Db>) -> Self {
        Self {
            repo: Repository::new(db),
        }
    }

    pub async fn write(&self, bytes: &[u8]) -> Result<RunBlobId> {
        let blob = Blob(Bytes::copy_from_slice(bytes));
        let id = blob.id();
        self.repo.put(&blob).await?;
        Ok(id)
    }

    pub async fn read(&self, id: &RunBlobId) -> Result<Option<Bytes>> {
        Ok(self.repo.get(id).await?.map(|blob| blob.0))
    }

    pub async fn exists(&self, id: &RunBlobId) -> Result<bool> {
        self.repo.exists(id).await
    }

    pub(crate) async fn list(&self) -> Result<Vec<RunBlobId>> {
        let mut stream = self.repo.scan_ids_stream();
        let mut ids = Vec::new();
        while let Some(result) = stream.next().await {
            match result {
                Ok(id) => ids.push(id),
                Err(Error::KeyParse(_)) => {}
                Err(err) => return Err(err),
            }
        }
        ids.sort();
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use bytes::Bytes;
    use fabro_types::RunBlobId;
    use object_store::memory::InMemory;

    use super::BlobStore;
    use crate::Database;
    use crate::keys::SlateKey;

    async fn store() -> Arc<BlobStore> {
        let db = Database::new(
            Arc::new(InMemory::new()),
            "",
            Duration::from_millis(1),
            None,
        );
        db.blobs().await.unwrap()
    }

    async fn raw_store(name: &str) -> (Arc<slatedb::Db>, BlobStore) {
        let raw_db = Arc::new(
            slatedb::Db::open(name, Arc::new(InMemory::new()))
                .await
                .unwrap(),
        );
        let store = BlobStore::new(Arc::clone(&raw_db));
        (raw_db, store)
    }

    #[tokio::test]
    async fn writes_reads_and_checks_existence() {
        let store = store().await;
        let bytes = b"hello world";
        let id = store.write(bytes).await.unwrap();

        assert_eq!(
            store.read(&id).await.unwrap(),
            Some(Bytes::from_static(bytes))
        );
        assert_eq!(store.write(bytes).await.unwrap(), id);
        assert!(store.exists(&id).await.unwrap());
        assert!(!store.exists(&RunBlobId::new(b"missing")).await.unwrap());
    }

    #[tokio::test]
    async fn empty_blobs_round_trip() {
        let store = store().await;
        let id = store.write(b"").await.unwrap();

        assert_eq!(store.read(&id).await.unwrap(), Some(Bytes::new()));
    }

    #[tokio::test]
    async fn list_returns_sorted_ids_and_handles_empty_store() {
        let store = store().await;
        assert!(store.list().await.unwrap().is_empty());

        let first_id = store.write(br#"{"z":1}"#).await.unwrap();
        let second_id = store.write(br#"{"a":1}"#).await.unwrap();
        let mut expected = vec![first_id, second_id];
        expected.sort();

        assert_eq!(store.list().await.unwrap(), expected);
    }

    #[tokio::test]
    async fn list_skips_malformed_blob_ids() {
        let (raw_db, store) = raw_store("blob-store-list-tests").await;
        let id = store.write(b"valid").await.unwrap();

        raw_db
            .put(
                SlateKey::new("blobs").with("sha256").with("not-a-blob-id"),
                b"malformed",
            )
            .await
            .unwrap();

        assert_eq!(store.list().await.unwrap(), vec![id]);
    }

    #[tokio::test]
    async fn raw_db_reads_exact_blob_bytes() {
        let (raw_db, store) = raw_store("blob-store-tests").await;
        let bytes = b"{\"ok\":true}";
        let id = store.write(bytes).await.unwrap();

        let saved = raw_db
            .get(SlateKey::new("blobs").with("sha256").with(id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.as_ref(), bytes);
    }
}
