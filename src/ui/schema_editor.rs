use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui::{self, ComboBox, RichText, TextEdit};

use crate::domain::error::GevoError;
use crate::domain::typesense_client::{SdkTypesenseClient, TypesenseClient};
use crate::ui::collections::format_count;
use crate::ui::schema_draft::{self, BASIC_TYPES, Request, SchemaDraft};
use crate::ui::status_chip::{ChipView, status_chip};
use crate::ui::task;

/// What the screen asks the app to do after a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    None,
    Back,
    /// The collection was created or altered; the list is stale.
    Done,
}

impl Outcome {
    /// Whether the collection list must be refetched. Leaving mid-apply counts: the request
    /// still runs, but nobody is left to watch its result.
    pub fn list_is_stale(self, was_applying: bool) -> bool {
        self == Outcome::Done || (self == Outcome::Back && was_applying)
    }
}

type Applying = Receiver<Result<(), GevoError>>;

/// State of the schema editor. Logic is here; `render` is thin glue.
pub struct SchemaEditorScreen {
    draft: SchemaDraft,
    mode: Mode,
    raw: String,
    raw_error: Option<String>,
    applying: Option<Applying>,
    error: Option<GevoError>,
}

/// Form edits the draft; Raw edits the request body directly, for what the form cannot express.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Form,
    Raw,
}

impl SchemaEditorScreen {
    pub fn new(draft: SchemaDraft) -> Self {
        Self {
            draft,
            mode: Mode::Form,
            raw: String::new(),
            raw_error: None,
            applying: None,
            error: None,
        }
    }

    pub fn draft(&self) -> &SchemaDraft {
        &self.draft
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Entering Raw starts from the form's request; leaving it discards the raw text.
    pub fn set_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        self.raw_error = None;
        self.raw = match mode {
            Mode::Raw => self.draft.request().body_json(),
            Mode::Form => String::new(),
        };
    }

    pub fn is_applying(&self) -> bool {
        self.applying.is_some()
    }

    /// Raw text is only checked when sent, so it never blocks the button.
    pub fn can_apply(&self) -> bool {
        !self.is_applying() && (self.mode == Mode::Raw || self.draft.problems().is_empty())
    }

    /// The request to send, or `None` with `raw_error` set when the raw text is unusable.
    pub fn build_request(&mut self) -> Option<Request> {
        if self.mode == Mode::Form {
            return Some(self.draft.request());
        }
        let parsed = match self.draft.alter_target() {
            Some(name) => Request::parse_alter(name, &self.raw),
            None => Request::parse_create(&self.raw),
        };
        match parsed {
            Ok(request) => {
                self.raw_error = None;
                Some(request)
            }
            Err(message) => {
                self.raw_error = Some(message);
                None
            }
        }
    }

    pub fn begin_apply(&mut self, pending: Applying) {
        self.error = None;
        self.applying = Some(pending);
    }

    /// Collects a finished apply: `Done` on success, otherwise the error is kept for display.
    pub fn poll(&mut self) -> Outcome {
        let Some(rx) = &self.applying else {
            return Outcome::None;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return Outcome::None,
            Err(TryRecvError::Disconnected) => Err(GevoError::Network(
                "request worker stopped unexpectedly".to_string(),
            )),
        };
        self.applying = None;
        match result {
            Ok(()) => Outcome::Done,
            Err(err) => {
                self.error = Some(err);
                Outcome::None
            }
        }
    }
}

/// Render glue; the logic above and in `schema_draft` is what is tested.
impl SchemaEditorScreen {
    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        client: Option<&Arc<SdkTypesenseClient>>,
    ) -> Outcome {
        let ctx = ui.ctx().clone();
        let mut outcome = self.poll();
        if self.is_applying() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.draft.is_create() {
                self.create_header(ui);
            } else {
                ui.heading(&self.draft.name);
                ui.weak(
                    "Existing fields are locked. Typesense can add or drop fields on a live \
                     collection, but cannot change a field's type or flags in place. To change \
                     one, drop it and add it again in the same update (the field is re-indexed).",
                );
            }
            ui.add_space(8.0);
            self.mode_toggle(ui);
            ui.add_space(8.0);
            match self.mode {
                Mode::Form => {
                    self.fields_grid(ui);
                    ui.add_space(4.0);
                    if ui.button("Add field").clicked() {
                        self.draft.add_blank();
                    }
                    ui.weak(
                        "Nested objects, embeddings and other advanced types are shown read-only.",
                    );
                    ui.add_space(12.0);
                    self.pending_panel(ui);
                    egui::CollapsingHeader::new("Preview request").show(ui, |ui| {
                        ui.label(RichText::new(self.draft.request().preview()).monospace());
                    });
                }
                Mode::Raw => self.raw_editor(ui),
            }
            ui.add_space(8.0);

            if let Some(err) = &self.error {
                ui.horizontal(|ui| {
                    status_chip(ui, &ChipView::from_error(err));
                    ui.label(err.to_string());
                });
            }
            ui.horizontal(|ui| {
                if ui.button("Back").clicked() {
                    outcome = Outcome::Back;
                }
                let apply = ui.add_enabled(
                    self.can_apply() && client.is_some(),
                    egui::Button::new(apply_label(&self.draft, self.mode)),
                );
                if apply.clicked()
                    && let Some(client) = client
                    && let Some(request) = self.build_request()
                {
                    self.begin_apply(spawn_apply(&ctx, client, request));
                }
                if self.is_applying() {
                    ui.spinner();
                }
            });
        });
        outcome
    }

    fn mode_toggle(&mut self, ui: &mut egui::Ui) {
        let mut mode = self.mode;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut mode, Mode::Form, "Form");
            ui.selectable_value(&mut mode, Mode::Raw, "Raw JSON");
        });
        self.set_mode(mode);
    }

    fn raw_editor(&mut self, ui: &mut egui::Ui) {
        ui.weak(
            "This is the request body. Edits here are sent as written and are not carried \
             back to the form.",
        );
        ui.add(
            TextEdit::multiline(&mut self.raw)
                .font(egui::TextStyle::Monospace)
                .code_editor()
                .desired_rows(14)
                .desired_width(f32::INFINITY),
        );
        if let Some(message) = &self.raw_error {
            ui.colored_label(ui.visuals().error_fg_color, message);
        }
    }

    fn create_header(&mut self, ui: &mut egui::Ui) {
        ui.heading("New collection");
        egui::Grid::new("create_header")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Name");
                ui.add(TextEdit::singleline(&mut self.draft.name).font(egui::TextStyle::Monospace));
                ui.end_row();
                ui.label("Default sorting field");
                ui.add(
                    TextEdit::singleline(&mut self.draft.default_sorting_field)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("optional, numeric field"),
                );
                ui.end_row();
            });
    }

    fn fields_grid(&mut self, ui: &mut egui::Ui) {
        let mut toggle = None;
        let mut remove = None;
        egui::Grid::new("schema_fields")
            .num_columns(7)
            .spacing([14.0, 6.0])
            .show(ui, |ui| {
                for h in ["Name", "Type", "Facet", "Index", "Optional", "Sort", ""] {
                    ui.label(RichText::new(h).weak());
                }
                ui.end_row();

                for f in self.draft.existing() {
                    let dropped = self.draft.is_dropped(&f.name);
                    let mut name = RichText::new(&f.name).monospace();
                    if dropped {
                        name = name.strikethrough().weak();
                    }
                    ui.label(name);
                    ui.label(RichText::new(&f.r#type).monospace());
                    for on in [
                        schema_draft::facet(f),
                        schema_draft::index(f),
                        schema_draft::optional(f),
                        schema_draft::sort(f),
                    ] {
                        ui.add_enabled(false, egui::Checkbox::without_text(&mut { on }));
                    }
                    let label = if dropped { "Undo" } else { "Drop" };
                    if ui.button(label).clicked() {
                        toggle = Some(f.name.clone());
                    }
                    ui.end_row();
                }

                for (i, f) in self.draft.added.iter_mut().enumerate() {
                    ui.add(
                        TextEdit::singleline(&mut f.name)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(160.0),
                    );
                    type_picker(ui, i, &mut f.r#type);
                    let mut facet = schema_draft::facet(f);
                    if ui.checkbox(&mut facet, "").changed() {
                        f.facet = Some(facet);
                    }
                    let mut index = schema_draft::index(f);
                    if ui.checkbox(&mut index, "").changed() {
                        f.index = Some(index);
                    }
                    let mut optional = schema_draft::optional(f);
                    if ui.checkbox(&mut optional, "").changed() {
                        f.optional = Some(optional);
                    }
                    let mut sort = schema_draft::sort(f);
                    if ui.checkbox(&mut sort, "").changed() {
                        f.sort = Some(sort);
                    }
                    if ui.button("Remove").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(name) = toggle {
            self.draft.toggle_drop(&name);
        }
        if let Some(i) = remove {
            self.draft.remove_added(i);
        }
    }

    fn pending_panel(&self, ui: &mut egui::Ui) {
        ui.heading("Pending changes");
        let lines = self.draft.pending_lines();
        if lines.is_empty() {
            ui.weak("Nothing yet.");
        }
        for line in &lines {
            ui.label(RichText::new(line).monospace());
        }
        if !self.draft.is_create() && self.draft.change_count() > 0 {
            ui.weak(format!(
                "Sent as one PATCH /collections/{}. Dropping a field removes it from the index \
                 for all {} documents.",
                self.draft.name,
                format_count(self.draft.documents()),
            ));
        }
        if self.draft.change_count() > 0 || self.draft.is_create() {
            for problem in self.draft.problems() {
                ui.colored_label(ui.visuals().error_fg_color, problem);
            }
        }
    }
}

fn type_picker(ui: &mut egui::Ui, row: usize, field_type: &mut String) {
    ComboBox::from_id_salt(("field_type", row))
        .selected_text(field_type.as_str())
        .width(110.0)
        .show_ui(ui, |ui| {
            for t in BASIC_TYPES {
                ui.selectable_value(field_type, t.to_owned(), t);
            }
        });
}

fn apply_label(draft: &SchemaDraft, mode: Mode) -> String {
    match (draft.is_create(), mode) {
        (true, _) => "Create collection".to_owned(),
        (false, Mode::Raw) => "Apply request".to_owned(),
        (false, Mode::Form) => {
            let n = draft.change_count();
            format!("Apply {n} change{}", if n == 1 { "" } else { "s" })
        }
    }
}

fn spawn_apply(
    ctx: &egui::Context,
    client: &Arc<SdkTypesenseClient>,
    request: Request,
) -> Applying {
    let client = Arc::clone(client);
    match request {
        Request::Alter { name, update } => task::run(ctx.clone(), async move {
            client.alter_collection(&name, update).await.map(drop)
        }),
        Request::Create(schema) => task::run(ctx.clone(), async move {
            client.create_collection(schema).await.map(drop)
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use typesense::models::{CollectionResponse, Field};

    use super::*;
    use crate::domain::error::TypesenseMsg;

    fn screen_with_change() -> SchemaEditorScreen {
        let c = CollectionResponse::new(
            "products".into(),
            vec![Field::new("legacy".into(), "string".into())],
            3,
            1,
        );
        let mut screen = SchemaEditorScreen::new(SchemaDraft::alter(&c));
        screen.draft.toggle_drop("legacy");
        screen
    }

    #[test]
    fn switching_to_raw_fills_the_text_from_the_draft() {
        let mut screen = screen_with_change();
        assert_eq!(screen.mode(), Mode::Form);
        screen.set_mode(Mode::Raw);
        assert_eq!(screen.mode(), Mode::Raw);
        assert!(screen.raw.contains("\"drop\": true"));
    }

    #[test]
    fn switching_back_to_form_keeps_the_draft_and_drops_raw_edits() {
        let mut screen = screen_with_change();
        screen.set_mode(Mode::Raw);
        screen.raw = "{}".into();
        screen.set_mode(Mode::Form);
        assert_eq!(screen.draft().change_count(), 1);
        screen.set_mode(Mode::Raw);
        assert!(screen.raw.contains("\"drop\": true"));
    }

    #[test]
    fn form_mode_builds_the_request_from_the_draft() {
        let mut screen = screen_with_change();
        assert!(matches!(
            screen.build_request(),
            Some(Request::Alter { ref name, .. }) if name == "products"
        ));
    }

    #[test]
    fn raw_mode_parses_the_edited_text() {
        let mut screen = screen_with_change();
        screen.set_mode(Mode::Raw);
        screen.raw = r#"{"fields":[{"name":"vec","type":"float[]","num_dim":3}]}"#.into();
        let Some(Request::Alter { update, .. }) = screen.build_request() else {
            panic!("expected an alter request");
        };
        assert_eq!(update.fields[0].name, "vec");
        assert!(screen.raw_error.is_none());
    }

    #[test]
    fn raw_mode_reports_bad_json_and_sends_nothing() {
        let mut screen = screen_with_change();
        screen.set_mode(Mode::Raw);
        screen.raw = "{oops".into();
        assert!(screen.build_request().is_none());
        assert!(
            screen
                .raw_error
                .as_deref()
                .unwrap()
                .starts_with("Invalid JSON")
        );
        screen.raw = r#"{"fields":[{"name":"a","type":"string"}]}"#.into();
        assert!(screen.build_request().is_some());
        assert!(screen.raw_error.is_none());
    }

    #[test]
    fn raw_create_mode_parses_a_full_schema() {
        let mut screen = SchemaEditorScreen::new(SchemaDraft::create());
        screen.set_mode(Mode::Raw);
        screen.raw = r#"{"name":"books","fields":[{"name":"t","type":"string"}]}"#.into();
        assert!(matches!(screen.build_request(), Some(Request::Create(_))));
    }

    #[test]
    fn raw_mode_can_apply_even_when_the_form_has_problems() {
        let c = CollectionResponse::new("products".into(), vec![], 0, 1);
        let mut screen = SchemaEditorScreen::new(SchemaDraft::alter(&c));
        assert!(!screen.can_apply());
        screen.set_mode(Mode::Raw);
        assert!(screen.can_apply());
    }

    #[test]
    fn the_list_is_stale_after_done_or_after_leaving_mid_apply() {
        assert!(Outcome::Done.list_is_stale(false));
        assert!(Outcome::Back.list_is_stale(true));
        assert!(!Outcome::Back.list_is_stale(false));
        assert!(!Outcome::None.list_is_stale(true));
    }

    #[test]
    fn apply_is_blocked_until_the_draft_is_valid() {
        let c = CollectionResponse::new("products".into(), vec![], 0, 1);
        let screen = SchemaEditorScreen::new(SchemaDraft::alter(&c));
        assert!(!screen.can_apply());
        assert!(screen_with_change().can_apply());
    }

    #[test]
    fn apply_is_blocked_while_one_is_running() {
        let mut screen = screen_with_change();
        let (_tx, rx) = channel();
        screen.begin_apply(rx);
        assert!(screen.is_applying());
        assert!(!screen.can_apply());
    }

    #[test]
    fn a_successful_apply_reports_done() {
        let mut screen = screen_with_change();
        let (tx, rx) = channel();
        screen.begin_apply(rx);
        assert_eq!(screen.poll(), Outcome::None);
        tx.send(Ok(())).unwrap();
        assert_eq!(screen.poll(), Outcome::Done);
        assert!(!screen.is_applying());
    }

    #[test]
    fn a_failed_apply_keeps_the_error_and_allows_a_retry() {
        let mut screen = screen_with_change();
        let (tx, rx) = channel();
        screen.begin_apply(rx);
        tx.send(Err(GevoError::Api(TypesenseMsg("bad field".into()))))
            .unwrap();
        assert_eq!(screen.poll(), Outcome::None);
        assert_eq!(screen.error.as_ref().map(GevoError::category), Some("API"));
        assert!(screen.can_apply());
    }

    #[test]
    fn a_dead_worker_is_reported_as_a_network_error() {
        let mut screen = screen_with_change();
        let (tx, rx) = channel();
        screen.begin_apply(rx);
        drop(tx);
        assert_eq!(screen.poll(), Outcome::None);
        assert_eq!(
            screen.error.as_ref().map(GevoError::category),
            Some("NETWORK")
        );
    }

    #[test]
    fn beginning_an_apply_clears_the_previous_error() {
        let mut screen = screen_with_change();
        screen.error = Some(GevoError::Network("x".into()));
        let (_tx, rx) = channel();
        screen.begin_apply(rx);
        assert!(screen.error.is_none());
    }

    #[test]
    fn apply_label_counts_changes_or_names_the_create() {
        let mut screen = screen_with_change();
        assert_eq!(apply_label(screen.draft(), Mode::Form), "Apply 1 change");
        screen.draft.add_blank();
        assert_eq!(apply_label(screen.draft(), Mode::Form), "Apply 2 changes");
        assert_eq!(
            apply_label(&SchemaDraft::create(), Mode::Form),
            "Create collection"
        );
        assert_eq!(apply_label(screen.draft(), Mode::Raw), "Apply request");
    }
}
