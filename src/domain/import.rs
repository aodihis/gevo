use std::io::{self, BufRead};
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use super::documents::{ImportAction, ImportLine};
use super::error::{GevoError, TypesenseMsg};
use super::typesense_client::TypesenseClient;

/// Lines sent per request: large enough to be quick, small enough to bound memory.
pub const CHUNK_LINES: usize = 1000;

/// A failed input line. `line` is 1-based in the source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowFailure {
    pub line: usize,
    pub id: Option<String>,
    pub error: String,
    /// The input line as Typesense echoed it back, kept so failures can be saved and retried.
    pub document: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub imported: usize,
    pub failures: Vec<RowFailure>,
    /// True when the run was cancelled before reaching the end of the file.
    pub cancelled: bool,
}

impl ImportSummary {
    pub fn failed(&self) -> usize {
        self.failures.len()
    }

    /// The failed documents as JSONL, ready to save and fix.
    pub fn failed_jsonl(&self) -> String {
        let mut out = String::new();
        for f in &self.failures {
            out.push_str(&f.document);
            out.push('\n');
        }
        out
    }
}

/// Reads up to `max` non-blank lines, each with its 1-based line number. `line_no` is the
/// number of lines consumed so far and is advanced past everything read, blank lines included.
pub fn read_chunk(
    reader: &mut impl BufRead,
    line_no: &mut usize,
    max: usize,
) -> io::Result<Vec<(usize, String)>> {
    let mut chunk = Vec::with_capacity(max.min(CHUNK_LINES));
    let mut buf = String::new();
    while chunk.len() < max {
        buf.clear();
        if reader.read_line(&mut buf)? == 0 {
            break;
        }
        *line_no += 1;
        let text = buf.trim_end_matches(['\n', '\r']);
        if !text.trim().is_empty() {
            chunk.push((*line_no, text.to_owned()));
        }
    }
    Ok(chunk)
}

/// The document's `id`, read from an echoed JSON line when it parses.
fn id_of(document: &str) -> Option<String> {
    match serde_json::from_str::<Value>(document).ok()?.get("id")? {
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

fn failure(line: usize, source: &str, result: &ImportLine) -> RowFailure {
    let document = result.document.clone().unwrap_or_else(|| source.to_owned());
    RowFailure {
        line,
        id: id_of(&document),
        error: result
            .error
            .clone()
            .unwrap_or_else(|| "Unknown error".to_owned()),
        document,
    }
}

/// Sends the reader's lines in chunks and collects per-row failures. `on_progress` receives
/// the number of lines sent so far. Stops between chunks when `cancel` is set.
pub async fn run_import<C: TypesenseClient>(
    client: &C,
    collection: &str,
    action: ImportAction,
    reader: &mut impl BufRead,
    chunk_lines: usize,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(usize),
) -> Result<ImportSummary, GevoError> {
    let mut summary = ImportSummary::default();
    let mut line_no = 0;
    let mut sent = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }
        let chunk = read_chunk(reader, &mut line_no, chunk_lines)
            .map_err(|e| GevoError::Api(TypesenseMsg(format!("Could not read the file: {e}"))))?;
        if chunk.is_empty() {
            break;
        }
        let body = chunk
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let results = client.import_documents(collection, body, action).await?;
        for (i, (line, text)) in chunk.iter().enumerate() {
            match results.get(i) {
                Some(r) if r.success => summary.imported += 1,
                Some(r) => summary.failures.push(failure(*line, text, r)),
                None => summary.failures.push(RowFailure {
                    line: *line,
                    id: id_of(text),
                    error: "Typesense returned no result for this line.".to_owned(),
                    document: text.clone(),
                }),
            }
        }
        sent += chunk.len();
        on_progress(sent);
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::domain::typesense_client::MockTypesenseClient;

    fn ok() -> ImportLine {
        ImportLine {
            success: true,
            error: None,
            document: None,
        }
    }

    fn bad(error: &str, document: &str) -> ImportLine {
        ImportLine {
            success: false,
            error: Some(error.to_owned()),
            document: Some(document.to_owned()),
        }
    }

    #[test]
    fn chunks_skip_blank_lines_but_keep_real_line_numbers() {
        let mut reader = Cursor::new("a\n\n  \nb\r\nc\n");
        let mut line_no = 0;
        let chunk = read_chunk(&mut reader, &mut line_no, 10).unwrap();
        assert_eq!(
            chunk,
            [
                (1, "a".to_owned()),
                (4, "b".to_owned()),
                (5, "c".to_owned())
            ]
        );
        assert_eq!(line_no, 5);
    }

    #[test]
    fn chunks_respect_the_size_limit_and_resume() {
        let mut reader = Cursor::new("1\n2\n3\n4\n5\n");
        let mut line_no = 0;
        let first = read_chunk(&mut reader, &mut line_no, 2).unwrap();
        let second = read_chunk(&mut reader, &mut line_no, 2).unwrap();
        let third = read_chunk(&mut reader, &mut line_no, 2).unwrap();
        assert_eq!(first.len() + second.len() + third.len(), 5);
        assert_eq!(second[0].0, 3);
        assert_eq!(third, [(5, "5".to_owned())]);
        assert!(read_chunk(&mut reader, &mut line_no, 2).unwrap().is_empty());
    }

    #[test]
    fn a_file_without_a_trailing_newline_is_fully_read() {
        let mut reader = Cursor::new("a\nb");
        let mut line_no = 0;
        assert_eq!(read_chunk(&mut reader, &mut line_no, 10).unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_clean_import_counts_every_row() {
        let mut client = MockTypesenseClient::new();
        client
            .expect_import_documents()
            .withf(|c, body, action| {
                c == "books"
                    && body == "{\"id\":\"1\"}\n{\"id\":\"2\"}"
                    && *action == ImportAction::Create
            })
            .times(1)
            .returning(|_, _, _| Ok(vec![ok(), ok()]));
        let mut reader = Cursor::new("{\"id\":\"1\"}\n{\"id\":\"2\"}\n");
        let mut seen = Vec::new();

        let summary = run_import(
            &client,
            "books",
            ImportAction::Create,
            &mut reader,
            10,
            &AtomicBool::new(false),
            |n| seen.push(n),
        )
        .await
        .unwrap();

        assert_eq!((summary.imported, summary.failed()), (2, 0));
        assert!(!summary.cancelled);
        assert_eq!(seen, [2]);
    }

    #[tokio::test]
    async fn failures_carry_line_numbers_ids_and_the_verbatim_error() {
        let mut client = MockTypesenseClient::new();
        client.expect_import_documents().returning(|_, _, _| {
            Ok(vec![
                ok(),
                bad(
                    "Field `price` must be a float.",
                    "{\"id\":\"sku-1\",\"price\":\"x\"}",
                ),
                bad("Bad JSON.", "{oops"),
            ])
        });
        let mut reader =
            Cursor::new("{\"id\":\"a\"}\n\n{\"id\":\"sku-1\",\"price\":\"x\"}\n{oops\n");

        let summary = run_import(
            &client,
            "books",
            ImportAction::Upsert,
            &mut reader,
            10,
            &AtomicBool::new(false),
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!(summary.imported, 1);
        assert_eq!(
            summary.failures,
            [
                RowFailure {
                    line: 3,
                    id: Some("sku-1".into()),
                    error: "Field `price` must be a float.".into(),
                    document: "{\"id\":\"sku-1\",\"price\":\"x\"}".into(),
                },
                RowFailure {
                    line: 4,
                    id: None,
                    error: "Bad JSON.".into(),
                    document: "{oops".into(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn a_short_response_marks_the_unanswered_lines_as_failed() {
        let mut client = MockTypesenseClient::new();
        client
            .expect_import_documents()
            .returning(|_, _, _| Ok(vec![ok()]));
        let mut reader = Cursor::new("{\"id\":\"1\"}\n{\"id\":\"2\"}\n");

        let summary = run_import(
            &client,
            "c",
            ImportAction::Create,
            &mut reader,
            10,
            &AtomicBool::new(false),
            |_| {},
        )
        .await
        .unwrap();

        assert_eq!((summary.imported, summary.failed()), (1, 1));
        assert_eq!(summary.failures[0].line, 2);
        assert_eq!(summary.failures[0].id.as_deref(), Some("2"));
    }

    #[tokio::test]
    async fn large_files_are_sent_in_several_requests_with_progress() {
        let mut client = MockTypesenseClient::new();
        client
            .expect_import_documents()
            .times(3)
            .returning(|_, body, _| Ok(body.lines().map(|_| ok()).collect()));
        let mut reader = Cursor::new(
            (1..=5)
                .map(|i| format!("{{\"id\":\"{i}\"}}\n"))
                .collect::<String>(),
        );
        let mut seen = Vec::new();

        let summary = run_import(
            &client,
            "c",
            ImportAction::Create,
            &mut reader,
            2,
            &AtomicBool::new(false),
            |n| seen.push(n),
        )
        .await
        .unwrap();

        assert_eq!(summary.imported, 5);
        assert_eq!(seen, [2, 4, 5]);
    }

    #[tokio::test]
    async fn a_request_error_stops_the_import_and_is_returned() {
        let mut client = MockTypesenseClient::new();
        client
            .expect_import_documents()
            .times(1)
            .returning(|_, _, _| Err(GevoError::Network("down".into())));
        let mut reader = Cursor::new("a\nb\nc\n");

        let err = run_import(
            &client,
            "c",
            ImportAction::Create,
            &mut reader,
            1,
            &AtomicBool::new(false),
            |_| {},
        )
        .await
        .unwrap_err();

        assert_eq!(err.category(), "NETWORK");
    }

    #[tokio::test]
    async fn cancelling_stops_before_the_next_chunk() {
        let cancel = AtomicBool::new(false);
        let mut client = MockTypesenseClient::new();
        client
            .expect_import_documents()
            .times(1)
            .returning(|_, body, _| Ok(body.lines().map(|_| ok()).collect()));
        let mut reader = Cursor::new("a\nb\nc\nd\n");

        let summary = run_import(
            &client,
            "c",
            ImportAction::Create,
            &mut reader,
            2,
            &cancel,
            |_| cancel.store(true, Ordering::Relaxed),
        )
        .await
        .unwrap();

        assert_eq!(summary.imported, 2);
        assert!(summary.cancelled);
    }

    #[test]
    fn failed_rows_are_saved_as_jsonl() {
        let summary = ImportSummary {
            failures: vec![
                RowFailure {
                    line: 1,
                    id: None,
                    error: "e".into(),
                    document: "{\"a\":1}".into(),
                },
                RowFailure {
                    line: 2,
                    id: None,
                    error: "e".into(),
                    document: "{oops".into(),
                },
            ],
            ..ImportSummary::default()
        };
        assert_eq!(summary.failed_jsonl(), "{\"a\":1}\n{oops\n");
        assert_eq!(ImportSummary::default().failed_jsonl(), "");
    }

    #[test]
    fn numeric_ids_are_read_too() {
        assert_eq!(id_of("{\"id\": 7}").as_deref(), Some("7"));
        assert_eq!(id_of("not json"), None);
        assert_eq!(id_of("{\"name\":\"x\"}"), None);
    }
}
