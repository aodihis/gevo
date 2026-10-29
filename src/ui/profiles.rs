use std::collections::HashMap;

use eframe::egui::{self, Color32, RichText};
use uuid::Uuid;

use crate::domain::connection_service::{ConnectionTestReport, StepStatus};
use crate::domain::error::GevoError;
use crate::domain::profile::ConnectionProfile;

/// Result of the most recent connection test of a profile; held in memory only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LastTest {
    Passed,
    Failed(GevoError),
}

impl LastTest {
    /// The first failed step decides; a report without failures counts as passed.
    pub fn from_report(report: &ConnectionTestReport) -> Self {
        report
            .steps()
            .into_iter()
            .find_map(|step| match step {
                StepStatus::Failed(err) => Some(LastTest::Failed(err.clone())),
                _ => None,
            })
            .unwrap_or(LastTest::Passed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipKind {
    Untested,
    Ok,
    Network,
    Tunnel,
    Auth,
    Api,
}

/// Label + state for the per-profile "Last test" cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastTestChip<'a> {
    pub kind: ChipKind,
    pub label: &'static str,
    pub detail: &'a str,
}

fn error_message(err: &GevoError) -> &str {
    match err {
        GevoError::Network(m) | GevoError::Tunnel(m) | GevoError::Auth(m) => m,
        GevoError::Api(m) => &m.0,
    }
}

pub fn last_test_chip(last: Option<&LastTest>) -> LastTestChip<'_> {
    match last {
        None => LastTestChip {
            kind: ChipKind::Untested,
            label: "NOT TESTED",
            detail: "",
        },
        Some(LastTest::Passed) => LastTestChip {
            kind: ChipKind::Ok,
            label: "OK",
            detail: "Reachable · admin key valid",
        },
        Some(LastTest::Failed(err)) => LastTestChip {
            kind: match err {
                GevoError::Network(_) => ChipKind::Network,
                GevoError::Tunnel(_) => ChipKind::Tunnel,
                GevoError::Auth(_) => ChipKind::Auth,
                GevoError::Api(_) => ChipKind::Api,
            },
            label: err.category(),
            detail: error_message(err),
        },
    }
}

/// One table row of the profiles screen.
#[derive(Debug, PartialEq, Eq)]
pub struct ProfileRow<'a> {
    pub profile: &'a ConnectionProfile,
    pub route: String,
    pub chip: LastTestChip<'a>,
}

fn route_label(profile: &ConnectionProfile) -> String {
    match &profile.ssh {
        Some(ssh) => format!("SSH · {}", ssh.host),
        None => "Direct".to_owned(),
    }
}

/// Rows sorted by name (case-insensitive), each joined with its last test result.
pub fn rows<'a>(
    profiles: &'a [ConnectionProfile],
    last_tests: &'a HashMap<Uuid, LastTest>,
) -> Vec<ProfileRow<'a>> {
    let mut rows: Vec<_> = profiles
        .iter()
        .map(|profile| ProfileRow {
            profile,
            route: route_label(profile),
            chip: last_test_chip(last_tests.get(&profile.id)),
        })
        .collect();
    rows.sort_by_cached_key(|r| r.profile.name.to_lowercase());
    rows
}

/// What the screen shows besides the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Banner {
    /// No profiles saved yet.
    Empty,
    /// Profiles exist but none is connected.
    Offline,
    /// A profile is connected.
    Hidden,
}

pub fn banner(profile_count: usize, connected: bool) -> Banner {
    match (profile_count, connected) {
        (0, _) => Banner::Empty,
        (_, false) => Banner::Offline,
        (_, true) => Banner::Hidden,
    }
}

/// Navigation requests raised by the profiles screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfilesAction {
    New,
    Edit(Uuid),
}

fn chip_colors(kind: ChipKind) -> (Color32, Color32) {
    let c = |hex: u32| Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
    match kind {
        ChipKind::Untested => (c(0x2A2C31), c(0x9A9890)),
        ChipKind::Ok => (c(0x1C3326), c(0x8FE0B0)),
        ChipKind::Network => (c(0x1B2A40), c(0xA9C8F7)),
        ChipKind::Tunnel => (c(0x2D2340), c(0xCDB2F5)),
        ChipKind::Auth => (c(0x3A2C14), c(0xF7C978)),
        ChipKind::Api => (c(0x3A1C16), c(0xF59A85)),
    }
}

/// Renders the profiles screen and returns the navigation the user asked for.
///
/// Render glue only -- exempt from unit tests per `AGENTS.md` §6. Test/Connect are shown
/// disabled until connection testing is wired into the app.
pub fn render(
    ui: &mut egui::Ui,
    profiles: &[ConnectionProfile],
    last_tests: &HashMap<Uuid, LastTest>,
    connected: bool,
) -> Option<ProfilesAction> {
    let mut action = None;

    ui.horizontal(|ui| {
        if ui.button("New connection").clicked() {
            action = Some(ProfilesAction::New);
        }
        ui.add_enabled(false, egui::Button::new("Test all"));
    });
    ui.add_space(8.0);
    ui.label("Profiles live on this machine. Admin keys and SSH secrets are kept in your OS keychain, never in the config file.");
    ui.add_space(8.0);

    match banner(profiles.len(), connected) {
        Banner::Empty => {
            ui.label("No connections yet. Create one to get started.");
            return action;
        }
        Banner::Offline => {
            ui.label(RichText::new("Offline · not connected to any server").weak());
        }
        Banner::Hidden => {}
    }

    egui::Grid::new("profiles_grid")
        .num_columns(5)
        .spacing([16.0, 10.0])
        .show(ui, |ui| {
            for h in ["Profile", "Endpoint", "Route", "Last test", "Actions"] {
                ui.label(RichText::new(h).weak());
            }
            ui.end_row();

            for row in rows(profiles, last_tests) {
                ui.label(RichText::new(&row.profile.name).strong());
                ui.label(RichText::new(&row.profile.url).monospace());
                ui.label(&row.route);
                ui.horizontal(|ui| {
                    let (bg, fg) = chip_colors(row.chip.kind);
                    ui.label(
                        RichText::new(row.chip.label)
                            .monospace()
                            .strong()
                            .color(fg)
                            .background_color(bg),
                    );
                    ui.label(row.chip.detail);
                });
                ui.horizontal(|ui| {
                    ui.add_enabled(false, egui::Button::new("Test"));
                    if ui.button("Edit").clicked() {
                        action = Some(ProfilesAction::Edit(row.profile.id));
                    }
                    ui.add_enabled(false, egui::Button::new("Connect"));
                });
                ui.end_row();
            }
        });

    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::connection_service::TunnelSteps;
    use crate::domain::error::TypesenseMsg;
    use crate::domain::profile::{SshAuthMethod, SshConfig};
    use crate::domain::ssh_tunnel::{TunnelFailure, TunnelStage};

    fn ssh_profile(name: &str, host: &str) -> ConnectionProfile {
        let mut p = ConnectionProfile::new(name, "http://10.0.0.1:8108");
        p.ssh = Some(SshConfig {
            host: host.into(),
            port: 22,
            username: "deploy".into(),
            auth: SshAuthMethod::Password,
            local_bind_port: 8108,
        });
        p
    }

    fn report(tunnel: TunnelSteps, health: StepStatus, auth: StepStatus) -> ConnectionTestReport {
        ConnectionTestReport {
            tunnel,
            health,
            auth,
        }
    }

    #[test]
    fn from_report_passes_when_all_steps_ok_or_skipped() {
        let r = report(TunnelSteps::skipped(), StepStatus::Ok, StepStatus::Ok);
        assert_eq!(LastTest::from_report(&r), LastTest::Passed);
    }

    #[test]
    fn from_report_uses_the_first_failed_step() {
        let tunnel = GevoError::Tunnel("ssh down".into());
        let r = report(
            TunnelSteps::failed(TunnelFailure::new(TunnelStage::Auth, tunnel.clone())),
            StepStatus::NotRun,
            StepStatus::NotRun,
        );
        assert_eq!(LastTest::from_report(&r), LastTest::Failed(tunnel));

        let auth = GevoError::Auth("bad key".into());
        let r = report(
            TunnelSteps::ok(),
            StepStatus::Ok,
            StepStatus::Failed(auth.clone()),
        );
        assert_eq!(LastTest::from_report(&r), LastTest::Failed(auth));
    }

    #[test]
    fn chip_without_a_test_is_untested() {
        let chip = last_test_chip(None);
        assert_eq!(chip.kind, ChipKind::Untested);
        assert_eq!(chip.detail, "");
    }

    #[test]
    fn chip_for_a_passed_test_is_ok() {
        let chip = last_test_chip(Some(&LastTest::Passed));
        assert_eq!((chip.kind, chip.label), (ChipKind::Ok, "OK"));
    }

    #[test]
    fn chip_maps_each_error_category_and_keeps_the_message_verbatim() {
        let cases = [
            (
                GevoError::Network("refused".into()),
                ChipKind::Network,
                "NETWORK",
                "refused",
            ),
            (
                GevoError::Tunnel("no route".into()),
                ChipKind::Tunnel,
                "TUNNEL",
                "no route",
            ),
            (
                GevoError::Auth("Forbidden".into()),
                ChipKind::Auth,
                "AUTH",
                "Forbidden",
            ),
            (
                GevoError::Api(TypesenseMsg("Not Found".into())),
                ChipKind::Api,
                "API",
                "Not Found",
            ),
        ];
        for (err, kind, label, detail) in cases {
            let last = LastTest::Failed(err);
            let chip = last_test_chip(Some(&last));
            assert_eq!((chip.kind, chip.label, chip.detail), (kind, label, detail));
        }
    }

    #[test]
    fn rows_are_sorted_by_name_ignoring_case() {
        let profiles = vec![
            ConnectionProfile::new("staging", "http://s"),
            ConnectionProfile::new("Prod", "http://p"),
            ConnectionProfile::new("local", "http://l"),
        ];
        let tests = HashMap::new();
        let names: Vec<_> = rows(&profiles, &tests)
            .iter()
            .map(|r| r.profile.name.as_str())
            .collect();
        assert_eq!(names, ["local", "Prod", "staging"]);
    }

    #[test]
    fn rows_label_the_route_and_join_last_test_by_id() {
        let direct = ConnectionProfile::new("a", "http://a");
        let tunneled = ssh_profile("b", "bastion-1");
        let mut tests = HashMap::new();
        tests.insert(tunneled.id, LastTest::Passed);
        let profiles = vec![direct, tunneled];

        let rows = rows(&profiles, &tests);

        assert_eq!(rows[0].route, "Direct");
        assert_eq!(rows[0].chip.kind, ChipKind::Untested);
        assert_eq!(rows[1].route, "SSH · bastion-1");
        assert_eq!(rows[1].chip.kind, ChipKind::Ok);
    }

    #[test]
    fn rows_of_no_profiles_is_empty() {
        assert!(rows(&[], &HashMap::new()).is_empty());
    }

    #[test]
    fn banner_decision() {
        assert_eq!(banner(0, false), Banner::Empty);
        assert_eq!(banner(0, true), Banner::Empty);
        assert_eq!(banner(2, false), Banner::Offline);
        assert_eq!(banner(2, true), Banner::Hidden);
    }

    #[test]
    fn every_chip_kind_has_distinct_colors() {
        let kinds = [
            ChipKind::Untested,
            ChipKind::Ok,
            ChipKind::Network,
            ChipKind::Tunnel,
            ChipKind::Auth,
            ChipKind::Api,
        ];
        let mut bgs: Vec<_> = kinds.iter().map(|k| chip_colors(*k).0).collect();
        bgs.sort_by_key(|c| c.to_array());
        bgs.dedup();
        assert_eq!(bgs.len(), kinds.len());
    }
}
