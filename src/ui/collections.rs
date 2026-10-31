use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui::{self, RichText};
use typesense::models::CollectionResponse;
use uuid::Uuid;

use crate::domain::error::GevoError;
use crate::domain::typesense_client::{SdkTypesenseClient, TypesenseClient};
use crate::ui::confirm_delete::{self, ConfirmDelete, ConfirmOutcome};
use crate::ui::session::{SessionSlot, SessionState};
use crate::ui::status_chip::{ChipView, status_chip};
use crate::ui::task;

/// One table row, formatted for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionRow {
    pub name: String,
    pub documents: String,
    pub fields: usize,
    pub default_sort: String,
    pub created: String,
}

pub fn row(c: &CollectionResponse) -> CollectionRow {
    CollectionRow {
        name: c.name.clone(),
        documents: format_count(c.num_documents),
        fields: c.fields.len(),
        default_sort: match c.default_sorting_field.as_deref() {
            Some(f) if !f.is_empty() => f.to_owned(),
            _ => "—".to_owned(),
        },
        created: format_date(c.created_at),
    }
}

/// `48210` -> `"48,210"`.
pub fn format_count(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Unix seconds as `"12 Mar 2026"` in UTC.
pub fn format_date(unix_secs: i64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    // Days-to-civil conversion (proleptic Gregorian), shifted so years start in March.
    let z = unix_secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{day:02} {} {year}", MONTHS[(month - 1) as usize])
}

/// Collections whose name contains `query`, ignoring case; everything for a blank query.
pub fn filter<'a>(items: &'a [CollectionResponse], query: &str) -> Vec<&'a CollectionResponse> {
    let needle = query.trim().to_lowercase();
    items
        .iter()
        .filter(|c| c.name.to_lowercase().contains(&needle))
        .collect()
}

/// The consequence line of the delete confirmation.
pub fn delete_detail(c: &CollectionResponse, connection: &str) -> String {
    format!(
        "This permanently deletes the collection and all {} documents on {connection}. It can't be undone.",
        format_count(c.num_documents)
    )
}

type Loading = Receiver<Result<Vec<CollectionResponse>, GevoError>>;
type Deleting = (String, Receiver<Result<(), GevoError>>);

enum ListState {
    Idle,
    Loading(Loading),
    Loaded(Vec<CollectionResponse>),
    Failed(GevoError),
}

/// Collections screen state. Logic is here; `render` is thin glue.
pub struct CollectionsScreen {
    list: ListState,
    loaded_for: Option<Uuid>,
    filter: String,
    confirm: Option<ConfirmDelete>,
    deleting: Option<Deleting>,
    action_error: Option<GevoError>,
}

impl Default for CollectionsScreen {
    fn default() -> Self {
        Self {
            list: ListState::Idle,
            loaded_for: None,
            filter: String::new(),
            confirm: None,
            deleting: None,
            action_error: None,
        }
    }
}

impl CollectionsScreen {
    /// True when the list was never fetched for this profile.
    pub fn needs_load(&self, profile: Uuid) -> bool {
        self.loaded_for != Some(profile)
    }

    pub fn begin_load(&mut self, profile: Uuid, pending: Loading) {
        if self.loaded_for != Some(profile) {
            // Both name a collection on the previous profile's server.
            self.confirm = None;
            self.deleting = None;
        }
        self.loaded_for = Some(profile);
        self.list = ListState::Loading(pending);
        self.action_error = None;
    }

    /// Forgets the list, e.g. when the connection goes away.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn collections(&self) -> &[CollectionResponse] {
        match &self.list {
            ListState::Loaded(items) => items,
            _ => &[],
        }
    }

    pub fn find(&self, name: &str) -> Option<&CollectionResponse> {
        self.collections().iter().find(|c| c.name == name)
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.list, ListState::Loading(_))
    }

    pub fn load_error(&self) -> Option<&GevoError> {
        match &self.list {
            ListState::Failed(err) => Some(err),
            _ => None,
        }
    }

    pub fn is_deleting(&self, name: &str) -> bool {
        self.deleting.as_ref().is_some_and(|(n, _)| n == name)
    }

    /// Collects finished loads and deletes; true while either is still running.
    pub fn poll(&mut self) -> bool {
        if let ListState::Loading(rx) = &self.list {
            match rx.try_recv() {
                Ok(Ok(items)) => self.list = ListState::Loaded(items),
                Ok(Err(err)) => self.list = ListState::Failed(err),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.list = ListState::Failed(worker_died()),
            }
        }
        if let Some((name, rx)) = &self.deleting {
            let finished = match rx.try_recv() {
                Ok(Ok(())) => {
                    if let ListState::Loaded(items) = &mut self.list {
                        items.retain(|c| &c.name != name);
                    }
                    true
                }
                Ok(Err(err)) => {
                    self.action_error = Some(err);
                    true
                }
                Err(TryRecvError::Empty) => false,
                Err(TryRecvError::Disconnected) => {
                    self.action_error = Some(worker_died());
                    true
                }
            };
            if finished {
                self.deleting = None;
            }
        }
        self.is_loading() || self.deleting.is_some()
    }

    /// Opens the confirmation for `name`; ignored if it is not in the list.
    pub fn ask_delete(&mut self, name: &str, connection: &str) {
        let Some(c) = self.collections().iter().find(|c| c.name == name) else {
            return;
        };
        self.confirm = Some(ConfirmDelete::new(name, delete_detail(c, connection)));
        self.action_error = None;
    }

    pub fn cancel_delete(&mut self) {
        self.confirm = None;
    }

    /// Closes the confirmation and returns the collection the user agreed to delete.
    pub fn take_confirmed(&mut self) -> Option<String> {
        self.confirm.take().map(|c| c.target().to_owned())
    }

    pub fn begin_delete(&mut self, name: String, pending: Receiver<Result<(), GevoError>>) {
        self.deleting = Some((name, pending));
    }
}

fn worker_died() -> GevoError {
    GevoError::Network("request worker stopped unexpectedly".to_string())
}

/// Render glue; the logic above is what is tested.
impl CollectionsScreen {
    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        session: &SessionSlot,
        connection: &str,
    ) -> Option<CollectionsAction> {
        let ctx = ui.ctx().clone();
        let Some(profile) = session.profile() else {
            ui.label("No active connection. Pick one from the switcher in the sidebar.");
            return None;
        };
        match session.state() {
            SessionState::None => {
                ui.label("No active connection. Pick one from the switcher in the sidebar.");
                return None;
            }
            SessionState::Connecting => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Connecting…");
                });
                return None;
            }
            SessionState::Failed(err) => {
                error_line(ui, err);
                return None;
            }
            SessionState::Ready(_) => {}
        }
        let client = session.client()?;
        let mut action = None;

        if self.needs_load(profile) {
            self.begin_load(profile, spawn_load(&ctx, &client));
        }
        if self.poll() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Filter collections")
                    .desired_width(240.0),
            );
            if ui
                .add_enabled(!self.is_loading(), egui::Button::new("Refresh"))
                .clicked()
            {
                self.begin_load(profile, spawn_load(&ctx, &client));
            }
            if self.is_loading() {
                ui.spinner();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("New collection").clicked() {
                    action = Some(CollectionsAction::New);
                }
            });
        });
        ui.add_space(8.0);

        if let Some(err) = self.load_error() {
            error_line(ui, err);
        }
        if let Some(err) = &self.action_error {
            error_line(ui, err);
        }

        let mut delete = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("collections_grid")
                .num_columns(6)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    for h in ["Name", "Documents", "Fields", "Default sort", "Created", ""] {
                        ui.label(RichText::new(h).weak());
                    }
                    ui.end_row();
                    for c in filter(self.collections(), &self.filter) {
                        let r = row(c);
                        let name = RichText::new(&r.name)
                            .monospace()
                            .color(ui.visuals().hyperlink_color);
                        if ui.link(name).clicked() {
                            action = Some(CollectionsAction::Open(r.name.clone()));
                        }
                        ui.label(RichText::new(&r.documents).monospace());
                        ui.label(RichText::new(r.fields.to_string()).monospace());
                        ui.label(&r.default_sort);
                        ui.label(&r.created);
                        let busy = self.is_deleting(&r.name);
                        ui.horizontal(|ui| {
                            if ui.button("Edit").clicked() {
                                action = Some(CollectionsAction::Edit(r.name.clone()));
                            }
                            if ui.button("Clone").clicked() {
                                action = Some(CollectionsAction::Clone(r.name.clone()));
                            }
                            if ui.add_enabled(!busy, egui::Button::new("Delete")).clicked() {
                                delete = Some(r.name.clone());
                            }
                        });
                        ui.end_row();
                    }
                });
        });
        if let Some(name) = delete {
            self.ask_delete(&name, connection);
        }

        let outcome = self
            .confirm
            .as_mut()
            .map(|confirm| confirm_delete::show(&ctx, confirm));
        match outcome {
            Some(ConfirmOutcome::Cancelled) => self.cancel_delete(),
            Some(ConfirmOutcome::Confirmed) => {
                if let Some(name) = self.take_confirmed() {
                    let pending = spawn_delete(&ctx, &client, name.clone());
                    self.begin_delete(name, pending);
                }
            }
            Some(ConfirmOutcome::Pending) | None => {}
        }
        action
    }
}

/// Where the user asked to go from the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionsAction {
    New,
    Edit(String),
    Clone(String),
    Open(String),
}

fn error_line(ui: &mut egui::Ui, err: &GevoError) {
    ui.horizontal(|ui| {
        status_chip(ui, &ChipView::from_error(err));
        ui.label(err.to_string());
    });
}

fn spawn_load(ctx: &egui::Context, client: &Arc<SdkTypesenseClient>) -> Loading {
    let client = Arc::clone(client);
    task::run(ctx.clone(), async move { client.list_collections().await })
}

fn spawn_delete(
    ctx: &egui::Context,
    client: &Arc<SdkTypesenseClient>,
    name: String,
) -> Receiver<Result<(), GevoError>> {
    let client = Arc::clone(client);
    task::run(ctx.clone(), async move {
        client.delete_collection(&name).await.map(drop)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    use typesense::models::Field;

    use crate::domain::error::TypesenseMsg;

    fn collection(name: &str, docs: i64) -> CollectionResponse {
        CollectionResponse::new(name.into(), vec![], docs, 1_700_000_000)
    }

    #[test]
    fn counts_get_thousands_separators() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000), "1,000");
        assert_eq!(format_count(48_210), "48,210");
        assert_eq!(format_count(1_234_567), "1,234,567");
        assert_eq!(format_count(-1_234), "-1,234");
    }

    #[test]
    fn dates_are_formatted_in_utc() {
        assert_eq!(format_date(0), "01 Jan 1970");
        assert_eq!(format_date(1_700_000_000), "14 Nov 2023");
        assert_eq!(format_date(951_782_400), "29 Feb 2000");
        assert_eq!(format_date(951_782_400 - 1), "28 Feb 2000");
        assert_eq!(format_date(-86_400), "31 Dec 1969");
    }

    #[test]
    fn row_formats_every_column() {
        let mut c = collection("products", 48_210);
        c.fields = vec![
            Field::new("name".into(), "string".into()),
            Field::new("price".into(), "float".into()),
        ];
        c.default_sorting_field = Some("price".into());
        assert_eq!(
            row(&c),
            CollectionRow {
                name: "products".into(),
                documents: "48,210".into(),
                fields: 2,
                default_sort: "price".into(),
                created: "14 Nov 2023".into(),
            }
        );
    }

    #[test]
    fn missing_or_empty_default_sort_shows_a_dash() {
        let mut c = collection("a", 0);
        assert_eq!(row(&c).default_sort, "—");
        c.default_sorting_field = Some(String::new());
        assert_eq!(row(&c).default_sort, "—");
    }

    #[test]
    fn filter_matches_substrings_ignoring_case() {
        let items = vec![collection("products", 1), collection("Support_Tickets", 2)];
        let names =
            |q| -> Vec<&str> { filter(&items, q).iter().map(|c| c.name.as_str()).collect() };
        assert_eq!(names("TICK"), ["Support_Tickets"]);
        assert_eq!(names("prod"), ["products"]);
        assert_eq!(names("zzz"), Vec::<&str>::new());
    }

    #[test]
    fn blank_filter_keeps_everything() {
        let items = vec![collection("a", 1), collection("b", 2)];
        assert_eq!(filter(&items, "").len(), 2);
        assert_eq!(filter(&items, "  ").len(), 2);
    }

    #[test]
    fn delete_detail_names_the_documents_and_connection() {
        let detail = delete_detail(&collection("products", 48_210), "prod-search");
        assert!(detail.contains("48,210 documents"));
        assert!(detail.contains("prod-search"));
    }

    fn loaded(names: &[&str]) -> CollectionsScreen {
        let mut screen = CollectionsScreen::default();
        let (tx, rx) = channel();
        screen.begin_load(Uuid::new_v4(), rx);
        tx.send(Ok(names.iter().map(|n| collection(n, 3)).collect()))
            .unwrap();
        screen.poll();
        screen
    }

    fn names(screen: &CollectionsScreen) -> Vec<&str> {
        screen
            .collections()
            .iter()
            .map(|c| c.name.as_str())
            .collect()
    }

    #[test]
    fn a_new_screen_needs_a_load_for_any_profile() {
        let screen = CollectionsScreen::default();
        assert!(screen.needs_load(Uuid::new_v4()));
        assert!(screen.collections().is_empty());
    }

    #[test]
    fn loading_then_loaded_exposes_the_collections() {
        let id = Uuid::new_v4();
        let mut screen = CollectionsScreen::default();
        let (tx, rx) = channel();
        screen.begin_load(id, rx);
        assert!(screen.is_loading());
        assert!(screen.poll());
        assert!(!screen.needs_load(id));
        assert!(screen.needs_load(Uuid::new_v4()));

        tx.send(Ok(vec![collection("a", 1)])).unwrap();
        assert!(!screen.poll());
        assert_eq!(names(&screen), ["a"]);
    }

    #[test]
    fn a_failed_load_keeps_the_error_for_display() {
        let mut screen = CollectionsScreen::default();
        let (tx, rx) = channel();
        screen.begin_load(Uuid::new_v4(), rx);
        tx.send(Err(GevoError::Auth("nope".into()))).unwrap();
        screen.poll();
        assert_eq!(screen.load_error(), Some(&GevoError::Auth("nope".into())));
        assert!(screen.collections().is_empty());
    }

    #[test]
    fn a_dead_load_worker_becomes_a_network_error() {
        let mut screen = CollectionsScreen::default();
        let (tx, rx) = channel();
        screen.begin_load(Uuid::new_v4(), rx);
        drop(tx);
        screen.poll();
        assert_eq!(
            screen.load_error().map(GevoError::category),
            Some("NETWORK")
        );
    }

    #[test]
    fn find_returns_a_loaded_collection_by_name() {
        let screen = loaded(&["a", "b"]);
        assert_eq!(screen.find("b").map(|c| c.name.as_str()), Some("b"));
        assert!(screen.find("ghost").is_none());
    }

    #[test]
    fn reset_forgets_the_list() {
        let mut screen = loaded(&["a"]);
        screen.reset();
        assert!(screen.collections().is_empty());
        assert!(screen.needs_load(Uuid::new_v4()));
    }

    #[test]
    fn ask_delete_requires_a_known_collection() {
        let mut screen = loaded(&["a"]);
        screen.ask_delete("ghost", "prod");
        assert_eq!(screen.take_confirmed(), None);
        screen.ask_delete("a", "prod");
        assert_eq!(screen.take_confirmed(), Some("a".to_string()));
        assert_eq!(screen.take_confirmed(), None);
    }

    #[test]
    fn cancel_delete_drops_the_confirmation() {
        let mut screen = loaded(&["a"]);
        screen.ask_delete("a", "prod");
        screen.cancel_delete();
        assert_eq!(screen.take_confirmed(), None);
    }

    #[test]
    fn a_successful_delete_removes_the_row_without_reloading() {
        let mut screen = loaded(&["a", "b"]);
        let (tx, rx) = channel();
        screen.begin_delete("a".into(), rx);
        assert!(screen.is_deleting("a"));
        assert!(screen.poll());

        tx.send(Ok(())).unwrap();
        assert!(!screen.poll());
        assert_eq!(names(&screen), ["b"]);
        assert!(!screen.is_deleting("a"));
    }

    #[test]
    fn switching_profiles_drops_a_delete_issued_for_the_old_one() {
        let mut screen = loaded(&["a"]);
        let (tx, rx) = channel();
        screen.begin_delete("a".into(), rx);
        screen.ask_delete("a", "prod");

        let (list_tx, list_rx) = channel();
        screen.begin_load(Uuid::new_v4(), list_rx);
        list_tx.send(Ok(vec![collection("a", 1)])).unwrap();
        let _ = tx.send(Ok(()));
        screen.poll();

        assert_eq!(names(&screen), ["a"]);
        assert!(!screen.is_deleting("a"));
        assert_eq!(screen.take_confirmed(), None);
    }

    #[test]
    fn refreshing_the_same_profile_keeps_a_running_delete() {
        let id = Uuid::new_v4();
        let mut screen = CollectionsScreen::default();
        let (tx, rx) = channel();
        screen.begin_load(id, rx);
        tx.send(Ok(vec![collection("a", 1)])).unwrap();
        screen.poll();
        let (_del_tx, del_rx) = channel();
        screen.begin_delete("a".into(), del_rx);

        let (_tx2, rx2) = channel();
        screen.begin_load(id, rx2);

        assert!(screen.is_deleting("a"));
    }

    #[test]
    fn a_failed_delete_keeps_the_row_and_records_the_error() {
        let mut screen = loaded(&["a"]);
        let (tx, rx) = channel();
        screen.begin_delete("a".into(), rx);
        tx.send(Err(GevoError::Api(TypesenseMsg("busy".into()))))
            .unwrap();
        screen.poll();
        assert_eq!(names(&screen), ["a"]);
        assert_eq!(
            screen.action_error.as_ref().map(GevoError::category),
            Some("API")
        );
    }

    #[test]
    fn a_dead_delete_worker_is_reported_and_cleared() {
        let mut screen = loaded(&["a"]);
        let (tx, rx) = channel();
        screen.begin_delete("a".into(), rx);
        drop(tx);
        assert!(!screen.poll());
        assert!(screen.action_error.is_some());
        assert!(!screen.is_deleting("a"));
    }

    #[test]
    fn asking_to_delete_again_clears_the_previous_error() {
        let mut screen = loaded(&["a"]);
        screen.action_error = Some(GevoError::Network("x".into()));
        screen.ask_delete("a", "prod");
        assert!(screen.action_error.is_none());
    }
}
