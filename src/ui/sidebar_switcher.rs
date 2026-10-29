use eframe::egui::{self, Color32, RichText};
use url::Url;
use uuid::Uuid;

use crate::domain::connection_service::ConnectionTestReport;
use crate::domain::profile::ConnectionProfile;

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
    /// `None` means the active profile has not been tested yet.
    pub fn from_report(report: Option<&ConnectionTestReport>) -> Self {
        match report {
            None => DotState::Idle,
            Some(r) if r.succeeded() => DotState::Connected,
            Some(_) => DotState::Failed,
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

/// Profile picker state: which profile is active and whether the list is expanded.
#[derive(Debug, Default)]
pub struct SwitcherState {
    pub active: Option<Uuid>,
    pub open: bool,
}

impl SwitcherState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    pub fn select(&mut self, id: Uuid) {
        self.active = Some(id);
        self.open = false;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitcherAction {
    Selected(Uuid),
    ManageConnections,
}

/// Render glue only; logic lives in the pure items above.
pub fn render(
    ui: &mut egui::Ui,
    state: &mut SwitcherState,
    profiles: &[ConnectionProfile],
    dot: DotState,
) -> Option<SwitcherAction> {
    let mut action = None;
    let (name, route) = match state.active_profile(profiles) {
        Some(p) => (p.name.as_str(), route_label(&p.url)),
        None => ("No connection", String::from("Select a profile")),
    };
    let head = ui.horizontal(|ui| {
        ui.label(RichText::new("●").color(dot.color()));
        ui.vertical(|ui| {
            ui.label(RichText::new(name).strong());
            ui.small(route);
        });
    });
    if head.response.interact(egui::Sense::click()).clicked() {
        state.toggle();
    }
    if state.open {
        for p in profiles {
            if ui
                .selectable_label(state.active == Some(p.id), &p.name)
                .clicked()
            {
                state.select(p.id);
                action = Some(SwitcherAction::Selected(p.id));
            }
        }
        if ui.button("Manage connections").clicked() {
            state.open = false;
            action = Some(SwitcherAction::ManageConnections);
        }
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::connection_service::{StepStatus, TunnelSteps};
    use crate::domain::error::GevoError;

    fn report(health: StepStatus) -> ConnectionTestReport {
        ConnectionTestReport {
            tunnel: TunnelSteps::skipped(),
            health,
            auth: StepStatus::Ok,
        }
    }

    #[test]
    fn untested_is_idle() {
        assert_eq!(DotState::from_report(None), DotState::Idle);
    }

    #[test]
    fn successful_report_is_connected() {
        let r = report(StepStatus::Ok);
        assert_eq!(DotState::from_report(Some(&r)), DotState::Connected);
    }

    #[test]
    fn failed_report_is_failed() {
        let r = report(StepStatus::Failed(GevoError::Network("boom".into())));
        assert_eq!(DotState::from_report(Some(&r)), DotState::Failed);
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
            active: None,
            open: true,
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
}
