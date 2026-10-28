use eframe::egui::{self, Color32, Frame};

/// Sidebar width, per the design handoff.
pub const SIDEBAR_WIDTH: f32 = 232.0;
/// Header bar height, per the design handoff.
pub const HEADER_HEIGHT: f32 = 52.0;

/// Renders the sidebar + header + central-content shell. `sidebar`, `header`, and `central`
/// each receive the `Ui` for their region; callers fill in actual nav/breadcrumb/screen content.
///
/// Render glue only (egui layout) — exempt from unit tests per `AGENTS.md` §6; the layout
/// constants above and the design-token lookups in `theme` are what's tested.
pub fn render(
    ui: &mut egui::Ui,
    sidebar_bg: Color32,
    sidebar: impl FnOnce(&mut egui::Ui),
    header: impl FnOnce(&mut egui::Ui),
    central: impl FnOnce(&mut egui::Ui),
) {
    egui::Panel::left("gevo_sidebar")
        .exact_size(SIDEBAR_WIDTH)
        .resizable(false)
        .frame(Frame::NONE.fill(sidebar_bg))
        .show(ui, sidebar);

    egui::CentralPanel::default_margins().show(ui, |ui| {
        egui::Panel::top("gevo_header")
            .exact_size(HEADER_HEIGHT)
            .resizable(false)
            .show(ui, header);
        central(ui);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_width_matches_design_handoff() {
        assert_eq!(SIDEBAR_WIDTH, 232.0);
    }

    #[test]
    fn header_height_matches_design_handoff() {
        assert_eq!(HEADER_HEIGHT, 52.0);
    }
}
