use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui::{self, RichText, TextEdit};
use serde_json::Value;
use typesense::models::{CollectionResponse, Field};

use crate::domain::documents::{DocumentQuery, SearchPage};
use crate::domain::error::GevoError;
use crate::domain::typesense_client::{SdkTypesenseClient, TypesenseClient};
use crate::ui::confirm_delete::{self, ConfirmDelete, ConfirmOutcome};
use crate::ui::export::{ExportState, export_to_file};
use crate::ui::session::{SessionSlot, SessionState};
use crate::ui::status_chip::{ChipView, status_chip};
use crate::ui::task;

pub const PER_PAGE_CHOICES: [u32; 4] = [10, 25, 50, 100];
const MAX_COLUMNS: usize = 6;
const MAX_CELL_CHARS: usize = 60;

/// `id` first, then the schema's fields; wildcard and flattened nested names are skipped.
/// A schemaless collection falls back to the keys of the first document.
pub fn columns(fields: &[Field], docs: &[Value]) -> Vec<String> {
    let usable = |name: &str| name != "id" && !name.contains(['*', '.']);
    let mut names: Vec<String> = fields
        .iter()
        .map(|f| f.name.as_str())
        .filter(|n| usable(n))
        .map(str::to_owned)
        .collect();
    if names.is_empty()
        && let Some(Value::Object(first)) = docs.first()
    {
        names = first.keys().filter(|k| usable(k)).cloned().collect();
    }
    let mut out = vec!["id".to_owned()];
    out.extend(names.into_iter().take(MAX_COLUMNS - 1));
    out
}

/// One table cell: scalars as text, arrays as `[a, b]`, objects as compact JSON, shortened.
pub fn cell_text(doc: &Value, column: &str) -> String {
    let text = match doc.get(column) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => {
            let parts: Vec<String> = items
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect();
            format!("[{}]", parts.join(", "))
        }
        Some(other) => other.to_string(),
    };
    shorten(&text, MAX_CELL_CHARS)
}

fn shorten(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let head: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{head}…")
}

/// `1–10 of 312`, or a plain message when there is nothing to show.
pub fn range_label(page: u32, per_page: u32, found: u64, shown: usize) -> String {
    if found == 0 || shown == 0 {
        return "0 results".to_owned();
    }
    let first = u64::from(page.saturating_sub(1)) * u64::from(per_page) + 1;
    let last = first + shown as u64 - 1;
    format!("{first}–{last} of {found}")
}

type Searching = Receiver<Result<SearchPage, GevoError>>;
type Deleting = (String, Receiver<Result<(), GevoError>>);

enum PageState {
    Idle,
    Loading(Searching),
    Loaded(SearchPage),
    Failed(GevoError),
}

/// Documents browser state for one collection. Logic is here; `render` is thin glue.
pub struct DocumentsScreen {
    collection: Option<CollectionResponse>,
    /// What the search bar currently holds.
    pub query: DocumentQuery,
    /// What the shown results came from, so paging ignores half-typed edits.
    applied: DocumentQuery,
    state: PageState,
    selected: Option<usize>,
    confirm: Option<ConfirmDelete>,
    deleting: Option<Deleting>,
    action_error: Option<GevoError>,
    export: ExportState,
}

impl Default for DocumentsScreen {
    fn default() -> Self {
        Self {
            collection: None,
            query: DocumentQuery::default(),
            applied: DocumentQuery::default(),
            state: PageState::Idle,
            selected: None,
            confirm: None,
            deleting: None,
            action_error: None,
            export: ExportState::default(),
        }
    }
}

impl DocumentsScreen {
    /// Starts fresh on `collection`; the first search is issued by the caller.
    pub fn open(&mut self, collection: CollectionResponse) {
        *self = Self {
            collection: Some(collection),
            ..Self::default()
        };
    }

    pub fn collection_name(&self) -> Option<&str> {
        self.collection.as_ref().map(|c| c.name.as_str())
    }

    pub fn needs_load(&self) -> bool {
        self.collection.is_some() && matches!(self.state, PageState::Idle)
    }

    /// Searches with the bar's values from page one; returns the query to send.
    pub fn submit(&mut self) -> DocumentQuery {
        self.query.page = 1;
        self.applied = self.query.clone();
        self.applied.clone()
    }

    /// Moves the shown results by `delta` pages; `None` when already at the edge.
    pub fn go_page(&mut self, delta: i32) -> Option<DocumentQuery> {
        let last = self.page()?.page_count(self.applied.per_page);
        let target = i64::from(self.applied.page) + i64::from(delta);
        let target = u32::try_from(target.clamp(1, i64::from(last))).ok()?;
        if target == self.applied.page {
            return None;
        }
        self.applied.page = target;
        self.query.page = target;
        Some(self.applied.clone())
    }

    /// Changes the page size, going back to page one.
    pub fn set_per_page(&mut self, per_page: u32) -> DocumentQuery {
        self.query.per_page = per_page;
        self.applied.per_page = per_page;
        self.applied.page = 1;
        self.query.page = 1;
        self.applied.clone()
    }

    pub fn applied(&self) -> &DocumentQuery {
        &self.applied
    }

    pub fn begin_search(&mut self, pending: Searching) {
        self.state = PageState::Loading(pending);
        self.selected = None;
        self.action_error = None;
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.state, PageState::Loading(_))
    }

    pub fn page(&self) -> Option<&SearchPage> {
        match &self.state {
            PageState::Loaded(page) => Some(page),
            _ => None,
        }
    }

    pub fn load_error(&self) -> Option<&GevoError> {
        match &self.state {
            PageState::Failed(err) => Some(err),
            _ => None,
        }
    }

    pub fn columns(&self) -> Vec<String> {
        let fields = self.collection.as_ref().map_or(&[][..], |c| &c.fields[..]);
        let docs = self.page().map_or(&[][..], |p| &p.documents[..]);
        columns(fields, docs)
    }

    pub fn select(&mut self, index: usize) {
        if self.page().is_some_and(|p| index < p.documents.len()) {
            self.selected = Some(index);
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn selected_doc(&self) -> Option<&Value> {
        self.page()?.documents.get(self.selected?)
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.selected_doc()?.get("id")?.as_str()
    }

    pub fn selected_json(&self) -> Option<String> {
        serde_json::to_string_pretty(self.selected_doc()?).ok()
    }

    /// Collects a finished search or delete; true while either is still running.
    pub fn poll(&mut self) -> bool {
        if let PageState::Loading(rx) = &self.state {
            match rx.try_recv() {
                Ok(Ok(page)) => self.state = PageState::Loaded(page),
                Ok(Err(err)) => self.state = PageState::Failed(err),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.state = PageState::Failed(worker_died()),
            }
        }
        if let Some((id, rx)) = self.deleting.take() {
            match rx.try_recv() {
                Ok(Ok(())) => self.remove_document(&id),
                Ok(Err(err)) => self.action_error = Some(err),
                Err(TryRecvError::Empty) => self.deleting = Some((id, rx)),
                Err(TryRecvError::Disconnected) => self.action_error = Some(worker_died()),
            }
        }
        let exporting = self.export.poll();
        self.is_loading() || self.deleting.is_some() || exporting
    }

    /// After deletes emptied the shown page while hits remain, the query for the last real page.
    pub fn refetch_query(&mut self) -> Option<DocumentQuery> {
        let page = self.page()?;
        if !page.documents.is_empty() || page.found == 0 || self.deleting.is_some() {
            return None;
        }
        let last = page.page_count(self.applied.per_page);
        self.applied.page = self.applied.page.min(last);
        self.query.page = self.applied.page;
        Some(self.applied.clone())
    }

    fn remove_document(&mut self, id: &str) {
        if let PageState::Loaded(page) = &mut self.state {
            let before = page.documents.len();
            page.documents
                .retain(|d| d.get("id").and_then(Value::as_str) != Some(id));
            if page.documents.len() < before {
                page.found = page.found.saturating_sub(1);
            }
        }
        self.selected = None;
    }

    /// Opens the confirmation for the selected document.
    pub fn ask_delete(&mut self) {
        let Some(id) = self.selected_id().map(str::to_owned) else {
            return;
        };
        self.confirm = Some(ConfirmDelete::plain(
            &id,
            format!("Delete document {id}?"),
            "This permanently deletes the document. It can't be undone.",
            "Delete document",
        ));
        self.action_error = None;
    }

    pub fn cancel_delete(&mut self) {
        self.confirm = None;
    }

    /// Closes the confirmation and returns the id the user agreed to delete.
    pub fn take_confirmed(&mut self) -> Option<String> {
        self.confirm.take().map(|c| c.target().to_owned())
    }

    pub fn begin_delete(&mut self, id: String, pending: Receiver<Result<(), GevoError>>) {
        self.deleting = Some((id, pending));
    }

    pub fn is_deleting(&self) -> bool {
        self.deleting.is_some()
    }
}

fn worker_died() -> GevoError {
    GevoError::Network("request worker stopped unexpectedly".to_string())
}

/// Where the user asked to go from the documents screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentsAction {
    Import,
}

/// Render glue; the logic above is what is tested.
impl DocumentsScreen {
    pub fn render(&mut self, ui: &mut egui::Ui, session: &SessionSlot) -> Option<DocumentsAction> {
        let ctx = ui.ctx().clone();
        let Some(client) = session.client() else {
            if let SessionState::Failed(err) = session.state() {
                error_line(ui, err);
            } else {
                ui.label("Not connected.");
            }
            return None;
        };
        let Some(name) = self.collection_name().map(str::to_owned) else {
            ui.label("Pick a collection from the Collections screen.");
            return None;
        };
        if self.needs_load() {
            let query = self.submit();
            self.begin_search(spawn_search(&ctx, &client, &name, query));
        }
        if self.poll() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        let mut action = None;
        ui.horizontal(|ui| {
            ui.heading(&name);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Import JSONL").clicked() {
                    action = Some(DocumentsAction::Import);
                }
                let label = if self.applied.filter_by.trim().is_empty() {
                    "Export"
                } else {
                    "Export (filtered)"
                };
                if ui
                    .add_enabled(!self.export.is_running(), egui::Button::new(label))
                    .on_hover_text(
                        "Saves the documents matching filter_by as JSONL; all if it is empty",
                    )
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_file_name(format!("{name}.jsonl"))
                        .save_file()
                {
                    let filter = self.applied.filter_by.clone();
                    self.export
                        .begin(spawn_export(&ctx, &client, &name, filter, path));
                }
            });
        });
        let mut next = None;
        self.search_bar(ui, &mut next);
        ui.add_space(8.0);

        if let Some(err) = self.load_error() {
            error_line(ui, err);
        }
        if let Some(err) = &self.action_error {
            error_line(ui, err);
        }
        if let Some(err) = self.export.error() {
            error_line(ui, err);
        } else if let Some(message) = self.export.message() {
            ui.weak(message);
        }
        self.results_table(ui);
        self.pager(ui, &mut next);
        ui.separator();
        self.detail(ui);

        let outcome = self
            .confirm
            .as_mut()
            .map(|confirm| confirm_delete::show(&ctx, confirm));
        match outcome {
            Some(ConfirmOutcome::Cancelled) => self.cancel_delete(),
            Some(ConfirmOutcome::Confirmed) => {
                if let Some(id) = self.take_confirmed() {
                    let pending = spawn_delete(&ctx, &client, &name, id.clone());
                    self.begin_delete(id, pending);
                }
            }
            Some(ConfirmOutcome::Pending) | None => {}
        }

        if next.is_none() {
            next = self.refetch_query();
        }
        if let Some(query) = next {
            self.begin_search(spawn_search(&ctx, &client, &name, query));
        }
        action
    }

    fn search_bar(&mut self, ui: &mut egui::Ui, next: &mut Option<DocumentQuery>) {
        let mut run = false;
        egui::Grid::new("documents_query")
            .num_columns(4)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                for h in ["q", "query_by", "filter_by", "sort_by"] {
                    ui.label(RichText::new(h).weak().monospace());
                }
                ui.end_row();
                for field in [
                    &mut self.query.q,
                    &mut self.query.query_by,
                    &mut self.query.filter_by,
                    &mut self.query.sort_by,
                ] {
                    let edit = ui.add(
                        TextEdit::singleline(field)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(180.0),
                    );
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        run = true;
                    }
                }
                ui.end_row();
            });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.is_loading(), egui::Button::new("Run"))
                .clicked()
            {
                run = true;
            }
            if let Some(page) = self.page() {
                ui.weak(format!("{} found · {} ms", page.found, page.search_time_ms));
            }
            if self.is_loading() {
                ui.spinner();
            }
        });
        if run && !self.is_loading() {
            *next = Some(self.submit());
        }
    }

    fn results_table(&mut self, ui: &mut egui::Ui) {
        let columns = self.columns();
        let mut picked = None;
        egui::ScrollArea::both()
            .id_salt("documents_scroll")
            .max_height(320.0)
            .show(ui, |ui| {
                let Some(page) = self.page() else {
                    return;
                };
                egui::Grid::new("documents_grid")
                    .num_columns(columns.len())
                    .spacing([16.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for c in &columns {
                            ui.label(RichText::new(c).weak());
                        }
                        ui.end_row();
                        for (i, doc) in page.documents.iter().enumerate() {
                            for (n, c) in columns.iter().enumerate() {
                                let text = RichText::new(cell_text(doc, c)).monospace();
                                // Only the first cell is a selectable label; it picks the row.
                                if n == 0 {
                                    let on = self.selected == Some(i);
                                    if ui.selectable_label(on, text).clicked() {
                                        picked = Some(i);
                                    }
                                } else {
                                    ui.label(text);
                                }
                            }
                            ui.end_row();
                        }
                    });
            });
        if let Some(i) = picked {
            self.select(i);
        }
    }

    fn pager(&mut self, ui: &mut egui::Ui, next: &mut Option<DocumentQuery>) {
        let Some(page) = self.page() else {
            return;
        };
        let label = range_label(
            self.applied.page,
            self.applied.per_page,
            page.found,
            page.documents.len(),
        );
        let busy = self.is_loading();
        let mut per_page = self.applied.per_page;
        let mut delta = 0;
        ui.horizontal(|ui| {
            ui.label("Per page");
            for n in PER_PAGE_CHOICES {
                ui.selectable_value(&mut per_page, n, n.to_string());
            }
            ui.weak(label);
            if ui.add_enabled(!busy, egui::Button::new("‹")).clicked() {
                delta = -1;
            }
            if ui.add_enabled(!busy, egui::Button::new("›")).clicked() {
                delta = 1;
            }
        });
        if per_page != self.applied.per_page {
            *next = Some(self.set_per_page(per_page));
        } else if delta != 0 {
            *next = self.go_page(delta);
        }
    }

    fn detail(&mut self, ui: &mut egui::Ui) {
        let Some(json) = self.selected_json() else {
            ui.weak("Select a document to see it here.");
            return;
        };
        let id = self.selected_id().unwrap_or_default().to_owned();
        ui.horizontal(|ui| {
            ui.label(RichText::new(&id).monospace().strong());
            if ui.button("Copy").clicked() {
                ui.ctx().copy_text(json.clone());
            }
            if ui
                .add_enabled(!self.is_deleting(), egui::Button::new("Delete"))
                .clicked()
            {
                self.ask_delete();
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("document_json")
            .max_height(260.0)
            .show(ui, |ui| {
                ui.label(RichText::new(json).monospace());
            });
    }
}

fn error_line(ui: &mut egui::Ui, err: &GevoError) {
    ui.horizontal(|ui| {
        status_chip(ui, &ChipView::from_error(err));
        ui.label(err.to_string());
    });
}

fn spawn_search(
    ctx: &egui::Context,
    client: &Arc<SdkTypesenseClient>,
    collection: &str,
    query: DocumentQuery,
) -> Searching {
    let (client, collection) = (Arc::clone(client), collection.to_owned());
    task::run(ctx.clone(), async move {
        client.search_documents(&collection, &query).await
    })
}

fn spawn_export(
    ctx: &egui::Context,
    client: &Arc<SdkTypesenseClient>,
    collection: &str,
    filter_by: String,
    path: std::path::PathBuf,
) -> Receiver<Result<crate::ui::export::Exported, GevoError>> {
    let (client, collection) = (Arc::clone(client), collection.to_owned());
    task::run(ctx.clone(), async move {
        export_to_file(&*client, &collection, &filter_by, &path).await
    })
}

fn spawn_delete(
    ctx: &egui::Context,
    client: &Arc<SdkTypesenseClient>,
    collection: &str,
    id: String,
) -> Receiver<Result<(), GevoError>> {
    let (client, collection) = (Arc::clone(client), collection.to_owned());
    task::run(ctx.clone(), async move {
        client.delete_document(&collection, &id).await.map(drop)
    })
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use serde_json::json;

    use super::*;
    use crate::domain::error::TypesenseMsg;

    fn field(name: &str) -> Field {
        Field::new(name.to_owned(), "string".to_owned())
    }

    fn collection(fields: &[&str]) -> CollectionResponse {
        CollectionResponse::new(
            "products".into(),
            fields.iter().map(|n| field(n)).collect(),
            0,
            1,
        )
    }

    fn page_of(ids: &[&str], found: u64) -> SearchPage {
        SearchPage {
            found,
            out_of: found,
            page: 1,
            search_time_ms: 1,
            documents: ids.iter().map(|id| json!({"id": id, "name": id})).collect(),
        }
    }

    /// A screen showing `ids`, with `found` total hits.
    fn loaded(ids: &[&str], found: u64) -> DocumentsScreen {
        let mut screen = DocumentsScreen::default();
        screen.open(collection(&["name"]));
        screen.submit();
        let (tx, rx) = channel();
        screen.begin_search(rx);
        tx.send(Ok(page_of(ids, found))).unwrap();
        screen.poll();
        screen
    }

    #[test]
    fn columns_start_with_id_and_follow_the_schema() {
        let fields = [field("name"), field("id"), field("price")];
        assert_eq!(columns(&fields, &[]), ["id", "name", "price"]);
    }

    #[test]
    fn columns_skip_wildcards_and_nested_names_and_cap_the_count() {
        let fields = ["a", ".*", "specs.brand", "b", "c", "d", "e", "f", "g"].map(field);
        assert_eq!(columns(&fields, &[]), ["id", "a", "b", "c", "d", "e"]);
    }

    #[test]
    fn a_schemaless_collection_uses_the_first_documents_keys() {
        let docs = [json!({"id": "1", "title": "x", "year": 1999})];
        let cols = columns(&[field(".*")], &docs);
        assert_eq!(cols[0], "id");
        assert!(cols.contains(&"title".to_owned()) && cols.contains(&"year".to_owned()));
    }

    #[test]
    fn cells_render_scalars_arrays_and_objects() {
        let doc = json!({
            "name": "Arc", "price": 59.5, "in_stock": true, "gone": null,
            "tags": ["audio", 2], "specs": {"w": 1}
        });
        assert_eq!(cell_text(&doc, "name"), "Arc");
        assert_eq!(cell_text(&doc, "price"), "59.5");
        assert_eq!(cell_text(&doc, "in_stock"), "true");
        assert_eq!(cell_text(&doc, "gone"), "");
        assert_eq!(cell_text(&doc, "missing"), "");
        assert_eq!(cell_text(&doc, "tags"), "[audio, 2]");
        assert_eq!(cell_text(&doc, "specs"), "{\"w\":1}");
    }

    #[test]
    fn long_cells_are_shortened_with_an_ellipsis() {
        let doc = json!({"t": "x".repeat(100)});
        let text = cell_text(&doc, "t");
        assert_eq!(text.chars().count(), MAX_CELL_CHARS);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn range_labels_cover_pages_and_empty_results() {
        assert_eq!(range_label(1, 10, 312, 10), "1–10 of 312");
        assert_eq!(range_label(32, 10, 312, 2), "311–312 of 312");
        assert_eq!(range_label(1, 10, 0, 0), "0 results");
    }

    #[test]
    fn a_new_screen_has_nothing_to_load_until_opened() {
        let mut screen = DocumentsScreen::default();
        assert!(!screen.needs_load());
        screen.open(collection(&[]));
        assert!(screen.needs_load());
        assert_eq!(screen.collection_name(), Some("products"));
    }

    #[test]
    fn submit_goes_to_page_one_and_remembers_what_was_sent() {
        let mut screen = DocumentsScreen::default();
        screen.open(collection(&[]));
        screen.query.q = "shoes".into();
        screen.query.page = 4;
        let sent = screen.submit();
        assert_eq!((sent.q.as_str(), sent.page), ("shoes", 1));
        screen.query.q = "typed but not run".into();
        assert_eq!(screen.applied().q, "shoes");
    }

    #[test]
    fn searching_then_loaded_exposes_the_page() {
        let screen = loaded(&["a", "b"], 2);
        assert_eq!(screen.page().map(|p| p.documents.len()), Some(2));
        assert!(!screen.needs_load());
    }

    #[test]
    fn a_failed_search_keeps_the_error() {
        let mut screen = DocumentsScreen::default();
        screen.open(collection(&[]));
        let (tx, rx) = channel();
        screen.begin_search(rx);
        tx.send(Err(GevoError::Api(TypesenseMsg("bad filter".into()))))
            .unwrap();
        screen.poll();
        assert_eq!(screen.load_error().map(GevoError::category), Some("API"));
        assert!(screen.page().is_none());
    }

    #[test]
    fn a_dead_search_worker_becomes_a_network_error() {
        let mut screen = DocumentsScreen::default();
        screen.open(collection(&[]));
        let (tx, rx) = channel();
        screen.begin_search(rx);
        drop(tx);
        screen.poll();
        assert_eq!(
            screen.load_error().map(GevoError::category),
            Some("NETWORK")
        );
    }

    #[test]
    fn paging_moves_within_bounds_and_stops_at_the_edges() {
        let mut screen = loaded(&["a"], 60);
        screen.set_per_page(25);
        assert!(screen.go_page(-1).is_none());
        assert_eq!(screen.go_page(1).map(|q| q.page), Some(2));
        assert_eq!(screen.go_page(1).map(|q| q.page), Some(3));
        assert!(screen.go_page(1).is_none());
        assert_eq!(screen.applied().page, 3);
    }

    #[test]
    fn paging_uses_the_applied_query_not_half_typed_edits() {
        let mut screen = loaded(&["a"], 100);
        screen.query.q = "edited".into();
        let next = screen.go_page(1).unwrap();
        assert_eq!(next.q, "*");
    }

    #[test]
    fn changing_the_page_size_returns_to_page_one() {
        let mut screen = loaded(&["a"], 500);
        screen.set_per_page(10);
        screen.go_page(3);
        let q = screen.set_per_page(50);
        assert_eq!((q.page, q.per_page), (1, 50));
    }

    #[test]
    fn selecting_a_row_exposes_its_document_and_json() {
        let mut screen = loaded(&["a", "b"], 2);
        screen.select(1);
        assert_eq!(screen.selected(), Some(1));
        assert_eq!(screen.selected_id(), Some("b"));
        assert!(screen.selected_json().unwrap().contains("\"name\": \"b\""));
    }

    #[test]
    fn selecting_out_of_range_is_ignored() {
        let mut screen = loaded(&["a"], 1);
        screen.select(5);
        assert_eq!(screen.selected(), None);
    }

    #[test]
    fn a_new_search_clears_the_selection() {
        let mut screen = loaded(&["a"], 1);
        screen.select(0);
        let (_tx, rx) = channel();
        screen.begin_search(rx);
        assert_eq!(screen.selected(), None);
    }

    #[test]
    fn delete_needs_a_selection_and_a_confirmation() {
        let mut screen = loaded(&["a"], 1);
        screen.ask_delete();
        assert_eq!(screen.take_confirmed(), None);
        screen.select(0);
        screen.ask_delete();
        screen.cancel_delete();
        assert_eq!(screen.take_confirmed(), None);
        screen.ask_delete();
        assert_eq!(screen.take_confirmed(), Some("a".to_string()));
    }

    #[test]
    fn a_successful_delete_drops_the_row_and_the_count() {
        let mut screen = loaded(&["a", "b"], 2);
        screen.select(0);
        let (tx, rx) = channel();
        screen.begin_delete("a".into(), rx);
        assert!(screen.is_deleting());
        assert!(screen.poll());
        tx.send(Ok(())).unwrap();
        assert!(!screen.poll());
        let page = screen.page().unwrap();
        assert_eq!((page.found, page.documents.len()), (1, 1));
        assert_eq!(screen.selected(), None);
    }

    #[test]
    fn deleting_the_last_row_of_a_later_page_asks_for_the_page_before() {
        let mut screen = loaded(&["a"], 26);
        screen.set_per_page(25);
        screen.go_page(1);
        let (tx, rx) = channel();
        screen.begin_search(rx);
        tx.send(Ok(page_of(&["z"], 26))).unwrap();
        screen.poll();
        let (dtx, drx) = channel();
        screen.begin_delete("z".into(), drx);
        dtx.send(Ok(())).unwrap();
        screen.poll();

        let query = screen.refetch_query().expect("a refetch");
        assert_eq!(query.page, 1);
        assert_eq!(screen.applied().page, 1);
    }

    #[test]
    fn no_refetch_while_the_page_still_has_rows() {
        let mut screen = loaded(&["a", "b"], 2);
        let (dtx, drx) = channel();
        screen.begin_delete("a".into(), drx);
        dtx.send(Ok(())).unwrap();
        screen.poll();
        assert!(screen.refetch_query().is_none());
    }

    #[test]
    fn no_refetch_when_the_collection_is_now_empty() {
        let mut screen = loaded(&["a"], 1);
        let (dtx, drx) = channel();
        screen.begin_delete("a".into(), drx);
        dtx.send(Ok(())).unwrap();
        screen.poll();
        assert!(screen.refetch_query().is_none());
    }

    #[test]
    fn a_failed_delete_keeps_the_row_and_records_the_error() {
        let mut screen = loaded(&["a"], 1);
        let (tx, rx) = channel();
        screen.begin_delete("a".into(), rx);
        tx.send(Err(GevoError::Api(TypesenseMsg("Not Found".into()))))
            .unwrap();
        screen.poll();
        assert_eq!(screen.page().unwrap().documents.len(), 1);
        assert_eq!(
            screen.action_error.as_ref().map(GevoError::category),
            Some("API")
        );
    }

    #[test]
    fn a_dead_delete_worker_is_reported_and_cleared() {
        let mut screen = loaded(&["a"], 1);
        let (tx, rx) = channel();
        screen.begin_delete("a".into(), rx);
        drop(tx);
        assert!(!screen.poll());
        assert!(screen.action_error.is_some());
        assert!(!screen.is_deleting());
    }

    #[test]
    fn opening_another_collection_forgets_everything() {
        let mut screen = loaded(&["a"], 1);
        screen.select(0);
        screen.open(collection(&[]));
        assert!(screen.page().is_none());
        assert_eq!(screen.selected(), None);
        assert!(screen.needs_load());
    }
}
