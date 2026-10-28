/// One entry per screen in `docs/design/DESIGN_HANDOFF.md`; iterate with `Route::iter()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::EnumIter)]
pub enum Route {
    Connections,
    ConnectionEdit,
    Health,
    Settings,
    Collections,
    SchemaEditor,
    ConfirmDelete,
    Documents,
    Import,
    ApiKeys,
    KeyCreated,
}

impl Route {
    /// Screen title, for the header breadcrumb and nav labels.
    pub fn title(self) -> &'static str {
        match self {
            Route::Connections => "Connections",
            Route::ConnectionEdit => "Edit Connection",
            Route::Health => "Health",
            Route::Settings => "Settings",
            Route::Collections => "Collections",
            Route::SchemaEditor => "Schema Editor",
            Route::ConfirmDelete => "Confirm Delete",
            Route::Documents => "Documents",
            Route::Import => "Import",
            Route::ApiKeys => "API Keys",
            Route::KeyCreated => "Key Created",
        }
    }
}

/// Holds the single currently-displayed route. The app starts on the connection
/// profiles screen per the handoff's navigation decision.
pub struct Router {
    current: Route,
}

impl Router {
    pub fn new() -> Self {
        Self {
            current: Route::Connections,
        }
    }

    pub fn current(&self) -> Route {
        self.current
    }

    pub fn go_to(&mut self, route: Route) {
        self.current = route;
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

/// Renders the (currently empty) placeholder screen for a route. Render glue only — exempt
/// from unit tests per `AGENTS.md` §6; `Route::title` and `Router` above are what's tested.
pub fn dispatch(route: Route, ui: &mut eframe::egui::Ui) {
    ui.heading(route.title());
    ui.label("Coming soon.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    #[test]
    fn every_route_has_a_non_empty_title() {
        for route in Route::iter() {
            assert!(!route.title().is_empty(), "{route:?} has an empty title");
        }
    }

    #[test]
    fn titles_are_unique_per_route() {
        let routes: Vec<Route> = Route::iter().collect();
        let mut titles: Vec<&str> = routes.iter().map(|r| r.title()).collect();
        titles.sort_unstable();
        titles.dedup();
        assert_eq!(titles.len(), routes.len());
    }

    #[test]
    fn router_starts_on_connections() {
        let router = Router::new();
        assert_eq!(router.current(), Route::Connections);
    }

    #[test]
    fn router_default_matches_new() {
        assert_eq!(Router::default().current(), Router::new().current());
    }

    #[test]
    fn go_to_changes_the_current_route() {
        let mut router = Router::new();
        router.go_to(Route::Settings);
        assert_eq!(router.current(), Route::Settings);
    }

    #[test]
    fn go_to_can_transition_through_multiple_routes() {
        let mut router = Router::new();
        router.go_to(Route::Collections);
        assert_eq!(router.current(), Route::Collections);
        router.go_to(Route::Documents);
        assert_eq!(router.current(), Route::Documents);
        router.go_to(Route::Connections);
        assert_eq!(router.current(), Route::Connections);
    }
}
