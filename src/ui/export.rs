use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::domain::error::{GevoError, TypesenseMsg};
use crate::domain::typesense_client::TypesenseClient;

/// What a finished export wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    pub path: PathBuf,
    pub documents: usize,
    pub bytes: usize,
}

/// Fetches the collection's documents (those matching `filter_by`, all if blank) and writes
/// them to `path` as JSONL.
pub async fn export_to_file<C: TypesenseClient>(
    client: &C,
    collection: &str,
    filter_by: &str,
    path: &Path,
) -> Result<Exported, GevoError> {
    let body = client.export_documents(collection, filter_by).await?;
    std::fs::write(path, &body).map_err(|e| {
        GevoError::Api(TypesenseMsg(format!(
            "Could not write {}: {e}",
            path.display()
        )))
    })?;
    Ok(Exported {
        path: path.to_owned(),
        documents: body.lines().filter(|l| !l.trim().is_empty()).count(),
        bytes: body.len(),
    })
}

type Pending = Receiver<Result<Exported, GevoError>>;

#[derive(Default)]
pub enum ExportState {
    #[default]
    Idle,
    Running(Pending),
    Done(Exported),
    Failed(GevoError),
}

impl ExportState {
    pub fn is_running(&self) -> bool {
        matches!(self, ExportState::Running(_))
    }

    pub fn begin(&mut self, pending: Pending) {
        *self = ExportState::Running(pending);
    }

    /// Collects a finished export; true while one is still running.
    pub fn poll(&mut self) -> bool {
        let ExportState::Running(rx) = self else {
            return false;
        };
        *self = match rx.try_recv() {
            Ok(Ok(done)) => ExportState::Done(done),
            Ok(Err(err)) => ExportState::Failed(err),
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => ExportState::Failed(GevoError::Network(
                "export worker stopped unexpectedly".to_string(),
            )),
        };
        false
    }

    /// One line for the status area; `None` while idle.
    pub fn message(&self) -> Option<String> {
        match self {
            ExportState::Idle => None,
            ExportState::Running(_) => Some("Exporting…".to_owned()),
            ExportState::Done(done) => Some(format!(
                "Exported {} documents to {}",
                crate::ui::collections::format_count(done.documents as i64),
                done.path.display()
            )),
            ExportState::Failed(err) => Some(err.to_string()),
        }
    }

    pub fn error(&self) -> Option<&GevoError> {
        match self {
            ExportState::Failed(err) => Some(err),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use super::*;
    use crate::domain::typesense_client::MockTypesenseClient;

    #[tokio::test]
    async fn the_exported_documents_are_written_verbatim() {
        let mut client = MockTypesenseClient::new();
        client
            .expect_export_documents()
            .withf(|c, filter| c == "books" && filter == "year:>2000")
            .returning(|_, _| Ok("{\"id\":\"1\"}\n{\"id\":\"2\"}\n".to_owned()));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("books.jsonl");

        let done = export_to_file(&client, "books", "year:>2000", &path)
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"id\":\"1\"}\n{\"id\":\"2\"}\n"
        );
        assert_eq!((done.documents, done.bytes), (2, 22));
        assert_eq!(done.path, path);
    }

    #[tokio::test]
    async fn an_empty_collection_writes_an_empty_file() {
        let mut client = MockTypesenseClient::new();
        client
            .expect_export_documents()
            .returning(|_, _| Ok(String::new()));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.jsonl");

        let done = export_to_file(&client, "c", "", &path).await.unwrap();

        assert_eq!(done.documents, 0);
        assert!(path.exists());
    }

    #[tokio::test]
    async fn a_server_error_is_returned_and_nothing_is_written() {
        let mut client = MockTypesenseClient::new();
        client
            .expect_export_documents()
            .returning(|_, _| Err(GevoError::Api(TypesenseMsg("Not Found".into()))));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.jsonl");

        let err = export_to_file(&client, "c", "", &path).await.unwrap_err();

        assert_eq!(err.category(), "API");
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn an_unwritable_path_is_reported() {
        let mut client = MockTypesenseClient::new();
        client
            .expect_export_documents()
            .returning(|_, _| Ok("{}\n".to_owned()));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("x.jsonl");

        let err = export_to_file(&client, "c", "", &path).await.unwrap_err();

        assert!(err.to_string().contains("Could not write"));
    }

    fn done() -> Exported {
        Exported {
            path: PathBuf::from("out.jsonl"),
            documents: 1200,
            bytes: 10,
        }
    }

    #[test]
    fn a_running_export_finishes_with_a_message() {
        let mut state = ExportState::default();
        assert_eq!(state.message(), None);
        let (tx, rx) = channel();
        state.begin(rx);
        assert!(state.is_running());
        assert!(state.poll());
        assert_eq!(state.message().as_deref(), Some("Exporting…"));

        tx.send(Ok(done())).unwrap();
        assert!(!state.poll());
        assert_eq!(
            state.message().as_deref(),
            Some("Exported 1,200 documents to out.jsonl")
        );
    }

    #[test]
    fn a_failed_export_keeps_the_error() {
        let mut state = ExportState::default();
        let (tx, rx) = channel();
        state.begin(rx);
        tx.send(Err(GevoError::Auth("no".into()))).unwrap();
        state.poll();
        assert_eq!(state.error().map(GevoError::category), Some("AUTH"));
        assert!(!state.is_running());
    }

    #[test]
    fn a_dead_worker_is_a_network_error() {
        let mut state = ExportState::default();
        let (tx, rx) = channel();
        state.begin(rx);
        drop(tx);
        assert!(!state.poll());
        assert_eq!(state.error().map(GevoError::category), Some("NETWORK"));
    }

    #[test]
    fn polling_while_idle_does_nothing() {
        let mut state = ExportState::default();
        assert!(!state.poll());
        assert!(!state.is_running());
    }
}
