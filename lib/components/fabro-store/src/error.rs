pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum Error {
    #[error("SlateDB error: {0}")]
    Slate(#[from] slatedb::Error),
    #[error("Object store error: {0}")]
    ObjectStore(#[from] object_store::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] sqlx::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid event payload: {0}")]
    InvalidEvent(String),
    #[error("event rejected by run projection: {source}")]
    EventRejected {
        #[source]
        source: Box<Self>,
    },
    #[error("Run not found: {0}")]
    RunNotFound(String),
    #[error("Session not found: {0}")]
    SessionNotFound(String),
    #[error("Session already exists: {0}")]
    SessionAlreadyExists(String),
    #[error("run store is read-only")]
    ReadOnly,
    #[error("event sequence limit of {max_seq} reached")]
    EventSequenceExhausted { max_seq: u32 },
    #[error("invalid key segment: {segment:?}")]
    InvalidKeySegment { segment: String },
    #[error("failed to parse key: {0}")]
    KeyParse(String),
    #[error("stored run summary {run_id} has inconsistent field {field}")]
    RunSummaryMismatch {
        run_id: String,
        field:  &'static str,
    },
    #[error(transparent)]
    InvalidTransition(#[from] fabro_types::InvalidTransition),
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// True for infrastructure failures (storage engine, object store,
    /// SQLite, I/O) where retrying the operation may succeed; false for
    /// structural errors that fail the same way every time.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Slate(_) | Self::ObjectStore(_) | Self::Sqlite(_) | Self::Io(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transience_and_kind_labels_are_structural() {
        let transient = [
            (
                Error::Slate(slatedb::Error::unavailable("test outage".to_string())),
                "slate",
            ),
            (
                Error::ObjectStore(object_store::Error::Generic {
                    store:  "test",
                    source: Box::new(std::io::Error::other("test outage")),
                }),
                "object_store",
            ),
            (Error::Sqlite(sqlx::Error::RowNotFound), "sqlite"),
            (Error::Io(std::io::Error::other("test outage")), "io"),
        ];
        for (error, kind) in transient {
            assert!(error.is_transient(), "{kind} should be transient");
            assert_eq!(<&'static str>::from(&error), kind);
        }

        let serde_error = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let permanent = [
            (
                Error::EventRejected {
                    source: Box::new(Error::ReadOnly),
                },
                "event_rejected",
            ),
            (
                Error::EventSequenceExhausted { max_seq: 10 },
                "event_sequence_exhausted",
            ),
            (Error::RunNotFound("missing".to_string()), "run_not_found"),
            (Error::ReadOnly, "read_only"),
            (Error::Serde(serde_error), "serde"),
        ];
        for (error, kind) in permanent {
            assert!(!error.is_transient(), "{kind} should be permanent");
            assert_eq!(<&'static str>::from(&error), kind);
        }
    }
}
