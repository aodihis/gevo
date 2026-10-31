use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText, TextEdit};
use strum::IntoEnumIterator;

use crate::domain::documents::ImportAction;
use crate::domain::error::{GevoError, TypesenseMsg};
use crate::domain::import::{CHUNK_LINES, ImportSummary, run_import};
use crate::domain::typesense_client::SdkTypesenseClient;
use crate::ui::collections::format_count;
use crate::ui::confirm_delete::{self, ConfirmDelete, ConfirmOutcome};
use crate::ui::session::SessionSlot;
use crate::ui::status_chip::{ChipView, status_chip};
use crate::ui::task;

/// What a running import reports back.
pub enum ImportEvent {
    Progress(usize),
    Finished(ImportSummary),
}

/// One line of help per action, from the design handoff.
pub fn action_help(action: ImportAction) -> &'static str {
    match action {
        ImportAction::Create => "Fail rows whose id already exists",
        ImportAction::Upsert => "Replaces existing documents. Asks to confirm.",
        ImportAction::Update => "Patch existing documents only",
        ImportAction::Emplace => "Create new, patch existing",
    }
}

/// `1.8 MB`, `12 KB`, `300 B`.
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

/// `3.4 s` below a minute, then `2 min 5 s`.
pub fn format_took(took: Duration) -> String {
    let secs = took.as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1} s")
    } else {
        let whole = took.as_secs();
        format!("{} min {} s", whole / 60, whole % 60)
    }
}

enum Phase {
    Idle,
    Running {
        events: Receiver<ImportEvent>,
        cancel: Arc<AtomicBool>,
        sent: usize,
        started: Instant,
    },
    Done {
        summary: ImportSummary,
        took: Duration,
    },
}

/// Import screen state for one collection. Logic is here; `render` is thin glue.
pub struct ImportScreen {
    collection: Option<String>,
    pub path: String,
    pub action: ImportAction,
    phase: Phase,
    confirm: Option<ConfirmDelete>,
    note: Option<String>,
}

impl Default for ImportScreen {
    fn default() -> Self {
        Self {
            collection: None,
            path: String::new(),
            action: ImportAction::Create,
            phase: Phase::Idle,
            confirm: None,
            note: None,
        }
    }
}

impl ImportScreen {
    pub fn open(&mut self, collection: &str) {
        *self = Self {
            collection: Some(collection.to_owned()),
            ..Self::default()
        };
    }

    pub fn collection(&self) -> Option<&str> {
        self.collection.as_deref()
    }

    pub fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running { .. })
    }

    pub fn can_start(&self) -> bool {
        !self.is_running() && !self.path.trim().is_empty()
    }

    /// Upsert replaces documents, so it is confirmed first.
    pub fn needs_confirmation(&self) -> bool {
        self.action == ImportAction::Upsert
    }

    pub fn begin(&mut self, events: Receiver<ImportEvent>, cancel: Arc<AtomicBool>) {
        self.phase = Phase::Running {
            events,
            cancel,
            sent: 0,
            started: Instant::now(),
        };
        self.note = None;
    }

    /// Asks a running import to stop after the request in flight.
    pub fn cancel(&self) {
        if let Phase::Running { cancel, .. } = &self.phase {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Applies pending events; true while the import is still running.
    pub fn poll(&mut self) -> bool {
        let Phase::Running {
            events,
            sent,
            started,
            ..
        } = &mut self.phase
        else {
            return false;
        };
        let mut finished = None;
        loop {
            match events.try_recv() {
                Ok(ImportEvent::Progress(n)) => *sent = n,
                Ok(ImportEvent::Finished(result)) => {
                    finished = Some(result);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = Some(ImportSummary {
                        error: Some(worker_died()),
                        ..ImportSummary::default()
                    });
                    break;
                }
            }
        }
        let took = started.elapsed();
        match finished {
            Some(summary) => self.phase = Phase::Done { summary, took },
            None => return true,
        }
        false
    }

    pub fn sent(&self) -> usize {
        match &self.phase {
            Phase::Running { sent, .. } => *sent,
            _ => 0,
        }
    }

    pub fn summary(&self) -> Option<(&ImportSummary, Duration)> {
        match &self.phase {
            Phase::Done { summary, took } => Some((summary, *took)),
            _ => None,
        }
    }

    /// The error that stopped the run, if any; the summary still holds what happened before it.
    pub fn error(&self) -> Option<&GevoError> {
        self.summary()?.0.error.as_ref()
    }

    /// Writes the failed rows as JSONL to `path`. No-op message when there are none.
    pub fn save_failed(&mut self, path: &Path) {
        let Some((summary, _)) = self.summary() else {
            return;
        };
        let jsonl = summary.failed_jsonl();
        self.note = Some(match std::fs::write(path, jsonl) {
            Ok(()) => format!("Saved failed rows to {}", path.display()),
            Err(e) => format!("Could not save: {e}"),
        });
    }

    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    pub fn ask_confirm(&mut self) {
        let Some(collection) = &self.collection else {
            return;
        };
        self.confirm = Some(ConfirmDelete::plain(
            collection,
            "Upsert into this collection?",
            "Documents with an existing id will be replaced entirely.",
            "Upsert",
        ));
    }

    pub fn cancel_confirm(&mut self) {
        self.confirm = None;
    }

    pub fn take_confirmed(&mut self) -> bool {
        self.confirm.take().is_some()
    }

    pub fn file_info(&self) -> Option<String> {
        let meta = std::fs::metadata(self.path.trim()).ok()?;
        meta.is_file().then(|| format_bytes(meta.len()))
    }
}

fn worker_died() -> GevoError {
    GevoError::Network("import worker stopped unexpectedly".to_string())
}

/// Opens the file and imports it on the shared runtime, reporting through `events`.
fn spawn_import(
    ctx: &egui::Context,
    client: &Arc<SdkTypesenseClient>,
    collection: &str,
    action: ImportAction,
    path: &str,
) -> (Receiver<ImportEvent>, Arc<AtomicBool>) {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let (client, collection, path) = (Arc::clone(client), collection.to_owned(), path.to_owned());
    let (flag, repaint) = (Arc::clone(&cancel), ctx.clone());
    // The task's own result channel is unused: everything goes through `events`.
    drop(task::run(ctx.clone(), async move {
        let result = import_file(&client, &collection, action, &path, &flag, &tx, &repaint).await;
        let _ = tx.send(ImportEvent::Finished(result));
        repaint.request_repaint();
    }));
    (rx, cancel)
}

async fn import_file(
    client: &SdkTypesenseClient,
    collection: &str,
    action: ImportAction,
    path: &str,
    cancel: &AtomicBool,
    events: &Sender<ImportEvent>,
    repaint: &egui::Context,
) -> ImportSummary {
    let file = match File::open(path.trim()) {
        Ok(file) => file,
        Err(e) => {
            let msg = format!("Could not open {}: {e}", path.trim());
            return ImportSummary {
                error: Some(GevoError::Api(TypesenseMsg(msg))),
                ..ImportSummary::default()
            };
        }
    };
    let mut reader = BufReader::new(file);
    run_import(
        client,
        collection,
        action,
        &mut reader,
        CHUNK_LINES,
        cancel,
        |sent| {
            let _ = events.send(ImportEvent::Progress(sent));
            repaint.request_repaint();
        },
    )
    .await
}

/// Render glue; the logic above is what is tested.
impl ImportScreen {
    /// Returns true when the user asked to go back to the documents.
    pub fn render(&mut self, ui: &mut egui::Ui, session: &SessionSlot) -> bool {
        let ctx = ui.ctx().clone();
        let (Some(client), Some(collection)) = (session.client(), self.collection.clone()) else {
            ui.label("Pick a collection from the Collections screen.");
            return false;
        };
        if self.poll() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        ui.heading(format!("Import into {collection}"));
        ui.add_space(8.0);
        let mut start = false;
        ui.horizontal(|ui| {
            ui.label("File");
            ui.add_enabled(
                !self.is_running(),
                TextEdit::singleline(&mut self.path)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("path to a .jsonl file")
                    .desired_width(380.0),
            );
            if ui
                .add_enabled(!self.is_running(), egui::Button::new("Browse…"))
                .clicked()
                && let Some(picked) = rfd::FileDialog::new()
                    .add_filter("JSON Lines", &["jsonl", "json", "ndjson"])
                    .pick_file()
            {
                self.path = picked.display().to_string();
            }
        });
        if let Some(size) = self.file_info() {
            ui.weak(size);
        }

        ui.add_space(8.0);
        ui.label("Action");
        let running = self.is_running();
        for action in ImportAction::iter() {
            ui.add_enabled_ui(!running, |ui| {
                ui.radio_value(&mut self.action, action, action.label())
                    .on_hover_text(action_help(action));
            });
        }
        ui.weak(action_help(self.action));

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if self.is_running() {
                if ui.button("Cancel").clicked() {
                    self.cancel();
                }
                ui.spinner();
                ui.label(format!("{} lines sent", format_count(self.sent() as i64)));
            } else if ui
                .add_enabled(self.can_start(), egui::Button::new("Run import"))
                .clicked()
            {
                start = true;
            }
        });

        if start {
            if self.needs_confirmation() {
                self.ask_confirm();
            } else {
                self.start(&ctx, &client, &collection);
            }
        }
        let outcome = self
            .confirm
            .as_mut()
            .map(|confirm| confirm_delete::show(&ctx, confirm));
        match outcome {
            Some(ConfirmOutcome::Cancelled) => self.cancel_confirm(),
            Some(ConfirmOutcome::Confirmed) => {
                if self.take_confirmed() {
                    self.start(&ctx, &client, &collection);
                }
            }
            Some(ConfirmOutcome::Pending) | None => {}
        }

        ui.add_space(12.0);
        self.results(ui);
        ui.add_space(8.0);
        ui.button("Back to documents").clicked()
    }

    fn start(&mut self, ctx: &egui::Context, client: &Arc<SdkTypesenseClient>, collection: &str) {
        let (events, cancel) = spawn_import(ctx, client, collection, self.action, &self.path);
        self.begin(events, cancel);
    }

    fn results(&mut self, ui: &mut egui::Ui) {
        if let Some(err) = self.error() {
            ui.horizontal(|ui| {
                status_chip(ui, &ChipView::from_error(err));
                ui.label(err.to_string());
            });
        }
        let Some((summary, took)) = self.summary() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.label(format!(
                "Imported {}",
                format_count(summary.imported as i64)
            ));
            ui.label(format!("Failed {}", format_count(summary.failed() as i64)));
            ui.label(format!("Took {}", format_took(took)));
            if summary.cancelled {
                ui.label(RichText::new("Cancelled").weak());
            }
        });
        let mut save = false;
        if summary.failed() > 0 {
            if ui.button("Save failed rows…").clicked() {
                save = true;
            }
            ui.add_space(4.0);
            ui.label(RichText::new(format!("Failed ({})", summary.failed())).strong());
            egui::ScrollArea::vertical()
                .id_salt("import_failures")
                .max_height(280.0)
                .show(ui, |ui| {
                    egui::Grid::new("import_failures_grid")
                        .num_columns(3)
                        .spacing([16.0, 4.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for h in ["Line", "id", "Typesense error"] {
                                ui.label(RichText::new(h).weak());
                            }
                            ui.end_row();
                            for f in &summary.failures {
                                ui.label(RichText::new(f.line.to_string()).monospace());
                                ui.label(RichText::new(f.id.as_deref().unwrap_or("—")).monospace());
                                ui.label(&f.error);
                                ui.end_row();
                            }
                        });
                });
        }
        if save
            && let Some(path) = rfd::FileDialog::new()
                .set_file_name("failed-rows.jsonl")
                .save_file()
        {
            self.save_failed(&path);
        }
        if let Some(note) = self.note() {
            ui.weak(note);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use super::*;
    use crate::domain::import::RowFailure;

    fn running() -> (ImportScreen, Sender<ImportEvent>, Arc<AtomicBool>) {
        let mut screen = ImportScreen::default();
        screen.open("books");
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        screen.begin(rx, Arc::clone(&cancel));
        (screen, tx, cancel)
    }

    fn summary_with_failure() -> ImportSummary {
        ImportSummary {
            imported: 3,
            failures: vec![RowFailure {
                line: 2,
                id: Some("x".into()),
                error: "bad".into(),
                document: "{\"id\":\"x\"}".into(),
            }],
            cancelled: false,
            error: None,
        }
    }

    #[test]
    fn bytes_are_formatted_in_binary_units() {
        assert_eq!(format_bytes(300), "300 B");
        assert_eq!(format_bytes(12 * 1024), "12 KB");
        assert_eq!(format_bytes(1_887_437), "1.8 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn durations_switch_to_minutes() {
        assert_eq!(format_took(Duration::from_millis(3400)), "3.4 s");
        assert_eq!(format_took(Duration::from_secs(125)), "2 min 5 s");
    }

    #[test]
    fn every_action_has_help_and_upsert_asks_first() {
        for action in ImportAction::iter() {
            assert!(!action_help(action).is_empty());
        }
        let mut screen = ImportScreen::default();
        assert!(!screen.needs_confirmation());
        screen.action = ImportAction::Upsert;
        assert!(screen.needs_confirmation());
    }

    #[test]
    fn starting_needs_a_path_and_no_import_in_flight() {
        let mut screen = ImportScreen::default();
        screen.open("books");
        assert!(!screen.can_start());
        screen.path = "  ".into();
        assert!(!screen.can_start());
        screen.path = "docs.jsonl".into();
        assert!(screen.can_start());
        let (tx, rx) = channel();
        screen.begin(rx, Arc::new(AtomicBool::new(false)));
        drop(tx);
        assert!(!screen.can_start());
    }

    #[test]
    fn progress_events_update_the_sent_count() {
        let (mut screen, tx, _) = running();
        tx.send(ImportEvent::Progress(1000)).unwrap();
        tx.send(ImportEvent::Progress(2000)).unwrap();
        assert!(screen.poll());
        assert_eq!(screen.sent(), 2000);
    }

    #[test]
    fn finishing_stores_the_summary() {
        let (mut screen, tx, _) = running();
        tx.send(ImportEvent::Finished(summary_with_failure()))
            .unwrap();
        assert!(!screen.poll());
        let (summary, _) = screen.summary().unwrap();
        assert_eq!((summary.imported, summary.failed()), (3, 1));
        assert!(!screen.is_running());
    }

    #[test]
    fn a_run_that_stopped_on_an_error_keeps_the_partial_results() {
        let (mut screen, tx, _) = running();
        let partial = ImportSummary {
            error: Some(GevoError::Auth("no".into())),
            ..summary_with_failure()
        };
        tx.send(ImportEvent::Finished(partial)).unwrap();
        screen.poll();
        assert_eq!(screen.error().map(GevoError::category), Some("AUTH"));
        let (summary, _) = screen.summary().unwrap();
        assert_eq!((summary.imported, summary.failed()), (3, 1));
    }

    #[test]
    fn a_dead_worker_is_reported() {
        let (mut screen, tx, _) = running();
        drop(tx);
        assert!(!screen.poll());
        assert_eq!(screen.error().map(GevoError::category), Some("NETWORK"));
    }

    #[test]
    fn cancel_raises_the_flag_only_while_running() {
        let (screen, _tx, cancel) = running();
        screen.cancel();
        assert!(cancel.load(Ordering::Relaxed));

        let idle = ImportScreen::default();
        idle.cancel();
    }

    #[test]
    fn failed_rows_are_written_as_jsonl() {
        let (mut screen, tx, _) = running();
        tx.send(ImportEvent::Finished(summary_with_failure()))
            .unwrap();
        screen.poll();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("failed.jsonl");

        screen.save_failed(&path);

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"id\":\"x\"}\n");
        assert!(screen.note().unwrap().starts_with("Saved failed rows"));
    }

    #[test]
    fn a_save_error_is_reported_not_raised() {
        let (mut screen, tx, _) = running();
        tx.send(ImportEvent::Finished(summary_with_failure()))
            .unwrap();
        screen.poll();
        let dir = tempfile::tempdir().unwrap();

        screen.save_failed(&dir.path().join("missing").join("f.jsonl"));

        assert!(screen.note().unwrap().starts_with("Could not save"));
    }

    #[test]
    fn upsert_confirmation_round_trips() {
        let mut screen = ImportScreen::default();
        screen.open("books");
        assert!(!screen.take_confirmed());
        screen.ask_confirm();
        screen.cancel_confirm();
        assert!(!screen.take_confirmed());
        screen.ask_confirm();
        assert!(screen.take_confirmed());
    }

    #[test]
    fn file_info_reports_the_size_of_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.jsonl");
        std::fs::write(&path, vec![b'x'; 2048]).unwrap();
        let mut screen = ImportScreen {
            path: path.display().to_string(),
            ..ImportScreen::default()
        };
        assert_eq!(screen.file_info().as_deref(), Some("2 KB"));
        screen.path = dir.path().display().to_string();
        assert_eq!(screen.file_info(), None);
        screen.path = "no/such/file".into();
        assert_eq!(screen.file_info(), None);
    }

    #[tokio::test]
    async fn a_missing_file_is_reported_in_the_summary() {
        let client = SdkTypesenseClient::new("http://localhost:1", "k").unwrap();
        let (tx, _rx) = channel();
        let summary = import_file(
            &client,
            "books",
            ImportAction::Create,
            "no/such/file.jsonl",
            &AtomicBool::new(false),
            &tx,
            &egui::Context::default(),
        )
        .await;
        let error = summary.error.expect("an error").to_string();
        assert!(error.contains("Could not open"), "{error}");
        assert_eq!(summary.imported, 0);
    }

    #[test]
    fn opening_another_collection_resets_the_screen() {
        let (mut screen, _tx, _) = running();
        screen.path = "x".into();
        screen.open("other");
        assert_eq!(screen.collection(), Some("other"));
        assert!(screen.path.is_empty());
        assert!(!screen.is_running());
    }
}
