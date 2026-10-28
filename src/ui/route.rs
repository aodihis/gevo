use crate::state::Route;

/// Renders the (currently empty) placeholder screen for a route. Render glue only — exempt
/// from unit tests per `AGENTS.md` §6; `Route::title` and `Router` (in `crate::state`) are
/// what's tested.
pub fn dispatch(route: Route, ui: &mut eframe::egui::Ui) {
    ui.heading(route.title());
    ui.label("Coming soon.");
}
