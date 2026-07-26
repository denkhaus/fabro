use crate::RunSummaryStore;
use crate::auth_session_store::AuthSessionStore;

pub(crate) async fn sqlite_auth_session_store() -> (tempfile::TempDir, AuthSessionStore) {
    let directory = tempfile::tempdir().unwrap();
    let database = fabro_db::Database::connect(directory.path().join("fabro.sqlite3"))
        .await
        .unwrap();
    database.migrate().await.unwrap();
    (directory, AuthSessionStore::new(database.clone_pool()))
}

pub(crate) async fn sqlite_summary_store() -> (tempfile::TempDir, RunSummaryStore) {
    let directory = tempfile::tempdir().unwrap();
    let database = fabro_db::Database::connect(directory.path().join("fabro.sqlite3"))
        .await
        .unwrap();
    database.migrate().await.unwrap();
    (directory, RunSummaryStore::new(database.clone_pool()))
}
