use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use eframe::egui::{self, Color32, CornerRadius, Popup, PopupCloseBehavior, RichText, Stroke};
use url::Url;
use uuid::Uuid;

use crate::domain::connection_service::{
    ConnectionService, ConnectionTestReport, SdkClientFactory,
};
use crate::domain::credential_store::KeyringCredentialStore;
use crate::domain::error::GevoError;
use crate::domain::profile::ConnectionProfile;
use crate::domain::russh_tunnel::RusshTunnel;
use crate::ui::profile_edit::RejectUnknownHostKey;
use crate::ui::profile_form::{FieldError, ProfileForm, run_test};
use crate::ui::profiles::LastTest;

const DOT_CONNECTED: Color32 = Color32::from_rgb(0x5F, 0xC0, 0x8A);
const DOT_IDLE: Color32 = Color32::from_rgb(0x8A, 0x8A, 0x85);
const DOT_FAILED: Color32 = Color32::from_rgb(0xE0, 0x6C, 0x5F);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DotState {
    Connected,
    Idle,
    Failed,
}

impl DotState {
    /// `None` means the profile has not been tested (or a test is in flight).
    pub fn from_last(last: Option<&LastTest>) -> Self {
        match last {
            None => DotState::Idle,
            Some(LastTest::Passed) => DotState::Connected,
            Some(LastTest::Failed(_)) => DotState::Failed,
        }
    }

    pub fn color(self) -> Color32 {
        match self {
            DotState::Connected => DOT_CONNECTED,
            DotState::Idle => DOT_IDLE,
            DotState::Failed => DOT_FAILED,
        }
    }
}

/// `host[:port]` of the profile URL, falling back to the raw string if it does not parse.
pub fn route_label(url: &str) -> String {
    match Url::parse(url) {
        Ok(u) => match (u.host_str(), u.port()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.to_owned(),
            _ => url.to_owned(),
        },
        Err(_) => url.to_owned(),
    }
}

/// Picker state. Active profile and test results live in memory only.
#[derive(Default)]
pub struct SwitcherState {
    pub active: Option<Uuid>,
    pub open: bool,
    pub results: HashMap<Uuid, LastTest>,
    running: HashMap<Uuid, Receiver<LastTest>>,
}

impl SwitcherState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn select(&mut self, id: Uuid) {
        self.active = Some(id);
        self.open = false;
    }

    pub fn is_running(&self, id: Uuid) -> bool {
        self.running.contains_key(&id)
    }

    /// Registers a test for `id`; `None` if one is already running for it.
    pub fn begin_test(&mut self, id: Uuid) -> Option<Sender<LastTest>> {
        if self.is_running(id) {
            return None;
        }
        let (tx, rx) = channel();
        self.results.remove(&id);
        self.running.insert(id, rx);
        Some(tx)
    }

    /// Collects finished tests; true while any are still running.
    pub fn poll(&mut self) -> bool {
        let results = &mut self.results;
        self.running.retain(|id, rx| match rx.try_recv() {
            Ok(result) => {
                results.insert(*id, result);
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => false,
        });
        !self.running.is_empty()
    }

    /// Dot for the active profile; grey while its test runs.
    pub fn dot(&self) -> DotState {
        DotState::from_last(self.active.and_then(|id| self.results.get(&id)))
    }

    /// The active profile, or `None` if nothing is selected or it was deleted.
    pub fn active_profile<'a>(
        &self,
        profiles: &'a [ConnectionProfile],
    ) -> Option<&'a ConnectionProfile> {
        let id = self.active?;
        profiles.iter().find(|p| p.id == id)
    }
}

/// A form that cannot be tested (e.g. no admin key in the keychain) counts as an auth failure.
fn outcome(result: Result<ConnectionTestReport, Vec<FieldError>>) -> LastTest {
    match result {
        Ok(report) => LastTest::from_report(&report),
        Err(errors) => {
            let messages: Vec<&str> = errors.iter().map(|e| e.message).collect();
            LastTest::Failed(GevoError::Auth(messages.join("; ")))
        }
    }
}

/// Runs the same service path as the connection edit screen on a worker thread.
fn spawn_test(profile: ConnectionProfile, tx: Sender<LastTest>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let form = ProfileForm::load(&profile, &KeyringCredentialStore::new());
        let service = ConnectionService::new(
            RusshTunnel::new(Arc::new(RejectUnknownHostKey)),
            SdkClientFactory,
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let _ = tx.send(outcome(runtime.block_on(run_test(&service, &form))));
        ctx.request_repaint();
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitcherAction {
    Selected(Uuid),
    ManageConnections,
}

/// Render glue only; logic lives in the pure items above. Selecting a profile starts its test.
pub fn render(
    ui: &mut egui::Ui,
    state: &mut SwitcherState,
    profiles: &[ConnectionProfile],
) -> Option<SwitcherAction> {
    if state.poll() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    let mut action = None;
    let dot = state.dot();
    let (name, route) = match state.active_profile(profiles) {
        Some(p) => (p.name.as_str(), route_label(&p.url)),
        None => ("No connection", String::from("Select a profile")),
    };
    let border = ui.visuals().widgets.noninteractive.bg_stroke.color;
    let head = egui::Frame::new()
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(10, 9))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").color(dot.color()));
                ui.vertical(|ui| {
                    ui.label(RichText::new(name).strong());
                    ui.small(route);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label("▾");
                });
            });
        })
        .response
        .interact(egui::Sense::click());
    if head.clicked() {
        state.toggle();
    }
    let mut open = state.open;
    Popup::from_response(&head)
        .open_bool(&mut open)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .width(head.rect.width())
        .show(|ui| {
            for p in profiles {
                if ui
                    .selectable_label(state.active == Some(p.id), &p.name)
                    .clicked()
                {
                    state.select(p.id);
                    if let Some(tx) = state.begin_test(p.id) {
                        spawn_test(p.clone(), tx, ui.ctx().clone());
                    }
                    action = Some(SwitcherAction::Selected(p.id));
                }
            }
            ui.separator();
            if ui.button("Manage connections").clicked() {
                state.close();
                action = Some(SwitcherAction::ManageConnections);
            }
        });
    if !open {
        state.close();
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::connection_service::StepStatus;
    use crate::ui::profile_form::Field;

    fn report(health: StepStatus) -> ConnectionTestReport {
        ConnectionTestReport {
            tunnel: StepStatus::Skipped,
            health,
            auth: StepStatus::Ok,
        }
    }

    #[test]
    fn untested_is_idle() {
        assert_eq!(DotState::from_last(None), DotState::Idle);
    }

    #[test]
    fn passed_test_is_connected() {
        assert_eq!(
            DotState::from_last(Some(&LastTest::Passed)),
            DotState::Connected
        );
    }

    #[test]
    fn failed_test_is_failed() {
        let t = LastTest::Failed(GevoError::Network("boom".into()));
        assert_eq!(DotState::from_last(Some(&t)), DotState::Failed);
    }

    #[test]
    fn dot_colors_are_distinct() {
        let c = [DotState::Connected, DotState::Idle, DotState::Failed].map(DotState::color);
        assert!(c[0] != c[1] && c[1] != c[2] && c[0] != c[2]);
    }

    #[test]
    fn route_label_keeps_host_and_port() {
        assert_eq!(
            route_label("https://ts.example.com:8108"),
            "ts.example.com:8108"
        );
    }

    #[test]
    fn route_label_drops_default_port_and_path() {
        assert_eq!(route_label("https://ts.example.com/x"), "ts.example.com");
    }

    #[test]
    fn route_label_falls_back_to_raw_input() {
        assert_eq!(route_label("not a url"), "not a url");
    }

    #[test]
    fn toggle_flips_open() {
        let mut s = SwitcherState::default();
        s.toggle();
        assert!(s.open);
        s.toggle();
        assert!(!s.open);
    }

    #[test]
    fn select_sets_active_and_closes() {
        let mut s = SwitcherState {
            open: true,
            ..Default::default()
        };
        let id = Uuid::new_v4();
        s.select(id);
        assert_eq!(s.active, Some(id));
        assert!(!s.open);
    }

    #[test]
    fn active_profile_resolves_or_none_when_missing() {
        let a = ConnectionProfile::new("A", "https://a.example.com");
        let profiles = vec![a.clone()];
        let mut s = SwitcherState::default();
        assert!(s.active_profile(&profiles).is_none());
        s.select(a.id);
        assert_eq!(s.active_profile(&profiles), Some(&a));
        s.select(Uuid::new_v4());
        assert!(s.active_profile(&profiles).is_none());
    }

    #[test]
    fn close_resets_open() {
        let mut s = SwitcherState {
            open: true,
            ..Default::default()
        };
        s.close();
        assert!(!s.open);
    }

    #[test]
    fn begin_test_clears_stale_result_and_marks_running() {
        let mut s = SwitcherState::default();
        let id = Uuid::new_v4();
        s.results.insert(id, LastTest::Passed);
        assert!(s.begin_test(id).is_some());
        assert!(s.is_running(id));
        assert!(!s.results.contains_key(&id));
    }

    #[test]
    fn begin_test_is_refused_while_the_same_profile_is_running() {
        let mut s = SwitcherState::default();
        let id = Uuid::new_v4();
        let _tx = s.begin_test(id);
        assert!(s.begin_test(id).is_none());
        assert!(s.begin_test(Uuid::new_v4()).is_some());
    }

    #[test]
    fn poll_records_result_and_stops_running() {
        let mut s = SwitcherState::default();
        let id = Uuid::new_v4();
        let tx = s.begin_test(id).unwrap();
        assert!(s.poll());
        tx.send(LastTest::Passed).unwrap();
        assert!(!s.poll());
        assert_eq!(s.results.get(&id), Some(&LastTest::Passed));
        assert!(!s.is_running(id));
    }

    #[test]
    fn poll_records_failure() {
        let mut s = SwitcherState::default();
        let id = Uuid::new_v4();
        let tx = s.begin_test(id).unwrap();
        let err = GevoError::Network("down".into());
        tx.send(LastTest::Failed(err.clone())).unwrap();
        s.poll();
        assert_eq!(s.results.get(&id), Some(&LastTest::Failed(err)));
    }

    #[test]
    fn invalid_profile_becomes_a_failed_result_with_the_messages() {
        let errors = vec![
            FieldError {
                field: Field::AdminKey,
                message: "Admin API key is required",
            },
            FieldError {
                field: Field::Host,
                message: "Host is required",
            },
        ];
        let expected = LastTest::Failed(GevoError::Auth(
            "Admin API key is required; Host is required".into(),
        ));
        assert_eq!(outcome(Err(errors)), expected);
    }

    #[test]
    fn report_outcome_follows_the_report() {
        assert_eq!(outcome(Ok(report(StepStatus::Ok))), LastTest::Passed);
    }

    #[test]
    fn poll_drops_a_worker_that_died_without_a_report() {
        let mut s = SwitcherState::default();
        let id = Uuid::new_v4();
        drop(s.begin_test(id));
        assert!(!s.poll());
        assert!(!s.is_running(id));
        assert!(s.results.is_empty());
    }

    #[test]
    fn dot_follows_the_active_profiles_result_and_is_idle_while_running() {
        let mut s = SwitcherState::default();
        assert_eq!(s.dot(), DotState::Idle);
        let id = Uuid::new_v4();
        s.select(id);
        s.results.insert(id, LastTest::Passed);
        assert_eq!(s.dot(), DotState::Connected);
        s.results
            .insert(id, LastTest::Failed(GevoError::Network("x".into())));
        assert_eq!(s.dot(), DotState::Failed);
        let _tx = s.begin_test(id);
        assert_eq!(s.dot(), DotState::Idle);
    }
}
