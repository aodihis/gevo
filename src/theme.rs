use eframe::egui::{Color32, Stroke, Theme, Visuals};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter, serde::Serialize, serde::Deserialize,
)]
pub enum Accent {
    Amber,
    Blue,
    Green,
    Violet,
}

impl Accent {
    /// Display name for the Settings screen's accent picker.
    pub fn name(self) -> &'static str {
        match self {
            Accent::Amber => "Amber",
            Accent::Blue => "Blue",
            Accent::Green => "Green",
            Accent::Violet => "Violet",
        }
    }
}

struct Tokens {
    page_bg: Color32,
    panel_bg: Color32,
    raised: Color32,
    selected: Color32,
    border: Color32,
    input_border: Color32,
    text: Color32,
    danger_text: Color32,
    accent_fill: Color32,
    accent_text: Color32,
}

/// Parses a `#RRGGBB` hex string from the design token tables.
fn hex(s: &str) -> Color32 {
    let s = s.trim_start_matches('#');
    let r = u8::from_str_radix(&s[0..2], 16).unwrap();
    let g = u8::from_str_radix(&s[2..4], 16).unwrap();
    let b = u8::from_str_radix(&s[4..6], 16).unwrap();
    Color32::from_rgb(r, g, b)
}

fn accent_hex(accent: Accent, mode: Theme) -> &'static str {
    match (accent, mode) {
        (Accent::Amber, Theme::Dark) => "#E8A33D",
        (Accent::Amber, Theme::Light) => "#A8650F",
        (Accent::Blue, Theme::Dark) => "#6EA8FE",
        (Accent::Blue, Theme::Light) => "#2F64C8",
        (Accent::Green, Theme::Dark) => "#5FC08A",
        (Accent::Green, Theme::Light) => "#23804D",
        (Accent::Violet, Theme::Dark) => "#B394F5",
        (Accent::Violet, Theme::Light) => "#6C46C9",
    }
}

fn tokens(mode: Theme, accent: Accent) -> Tokens {
    let accent_fill = hex(accent_hex(accent, mode));
    let accent_text = match mode {
        Theme::Dark => hex("#1A1408"),
        Theme::Light => hex("#FFFFFF"),
    };
    match mode {
        Theme::Dark => Tokens {
            page_bg: hex("#121315"),
            panel_bg: hex("#17181B"),
            raised: hex("#202226"),
            selected: hex("#26282D"),
            border: hex("#2C2E33"),
            input_border: hex("#34363C"),
            text: hex("#E7E5E0"),
            danger_text: hex("#F59A85"),
            accent_fill,
            accent_text,
        },
        Theme::Light => Tokens {
            page_bg: hex("#FBFAF8"),
            panel_bg: hex("#FFFFFF"),
            raised: hex("#F3F2EE"),
            selected: hex("#E4E2DB"),
            border: hex("#DDDBD4"),
            input_border: hex("#D3D1CA"),
            text: hex("#1D1D1B"),
            // Handoff defines danger fill/text only once (mode-invariant); light mode
            // has no separate danger token, so use the fill color directly for contrast on white.
            danger_text: hex("#B8402C"),
            accent_fill,
            accent_text,
        },
    }
}

/// Sidebar background, distinct from the general panel background
/// (handoff: sidebar bg #16171A vs panel bg #17181B in dark mode).
pub fn sidebar_bg(mode: Theme) -> Color32 {
    match mode {
        Theme::Dark => hex("#16171A"),
        Theme::Light => hex("#F3F2EE"),
    }
}

/// Builds an `egui::Visuals` from the Gevo design tokens for the given mode + accent.
pub fn build_visuals(mode: Theme, accent: Accent) -> Visuals {
    let t = tokens(mode, accent);
    let mut visuals = match mode {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
    };

    visuals.override_text_color = Some(t.text);
    visuals.window_fill = t.page_bg;
    visuals.panel_fill = t.panel_bg;

    visuals.widgets.inactive.bg_fill = t.raised;
    visuals.widgets.inactive.weak_bg_fill = t.raised;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, t.input_border);

    visuals.selection.bg_fill = t.accent_fill;
    visuals.selection.stroke = Stroke::new(1.0, t.accent_text);

    visuals.hyperlink_color = t.accent_fill;
    visuals.error_fg_color = t.danger_text;
    visuals.warn_fg_color = t.danger_text;
    visuals.extreme_bg_color = t.selected;
    visuals.faint_bg_color = t.raised;

    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses_rrggbb() {
        assert_eq!(hex("#121315"), Color32::from_rgb(0x12, 0x13, 0x15));
        assert_eq!(hex("FFFFFF"), Color32::from_rgb(0xFF, 0xFF, 0xFF));
    }

    #[test]
    fn dark_mode_uses_dark_page_and_panel_tokens() {
        let v = build_visuals(Theme::Dark, Accent::Amber);
        assert_eq!(v.window_fill, hex("#121315"));
        assert_eq!(v.panel_fill, hex("#17181B"));
        assert_eq!(v.override_text_color, Some(hex("#E7E5E0")));
    }

    #[test]
    fn light_mode_uses_light_page_and_panel_tokens() {
        let v = build_visuals(Theme::Light, Accent::Amber);
        assert_eq!(v.window_fill, hex("#FBFAF8"));
        assert_eq!(v.panel_fill, hex("#FFFFFF"));
        assert_eq!(v.override_text_color, Some(hex("#1D1D1B")));
    }

    #[test]
    fn every_accent_maps_to_correct_hex_in_dark_mode() {
        let cases = [
            (Accent::Amber, "#E8A33D"),
            (Accent::Blue, "#6EA8FE"),
            (Accent::Green, "#5FC08A"),
            (Accent::Violet, "#B394F5"),
        ];
        for (accent, expected) in cases {
            let v = build_visuals(Theme::Dark, accent);
            assert_eq!(v.selection.bg_fill, hex(expected), "{accent:?}");
        }
    }

    #[test]
    fn every_accent_maps_to_correct_hex_in_light_mode() {
        let cases = [
            (Accent::Amber, "#A8650F"),
            (Accent::Blue, "#2F64C8"),
            (Accent::Green, "#23804D"),
            (Accent::Violet, "#6C46C9"),
        ];
        for (accent, expected) in cases {
            let v = build_visuals(Theme::Light, accent);
            assert_eq!(v.selection.bg_fill, hex(expected), "{accent:?}");
        }
    }

    #[test]
    fn accent_text_is_dark_in_dark_mode_and_white_in_light_mode() {
        let dark = build_visuals(Theme::Dark, Accent::Blue);
        let light = build_visuals(Theme::Light, Accent::Blue);
        assert_eq!(dark.selection.stroke.color, hex("#1A1408"));
        assert_eq!(light.selection.stroke.color, hex("#FFFFFF"));
    }

    #[test]
    fn danger_text_differs_between_modes() {
        let dark = build_visuals(Theme::Dark, Accent::Amber);
        let light = build_visuals(Theme::Light, Accent::Amber);
        assert_eq!(dark.error_fg_color, hex("#F59A85"));
        assert_eq!(light.error_fg_color, hex("#B8402C"));
    }

    #[test]
    fn faint_bg_color_is_a_background_token_not_a_text_color() {
        let dark = build_visuals(Theme::Dark, Accent::Amber);
        let light = build_visuals(Theme::Light, Accent::Amber);
        assert_eq!(dark.faint_bg_color, hex("#202226"));
        assert_eq!(light.faint_bg_color, hex("#F3F2EE"));
    }

    #[test]
    fn sidebar_bg_matches_dark_and_light_tokens() {
        assert_eq!(sidebar_bg(Theme::Dark), hex("#16171A"));
        assert_eq!(sidebar_bg(Theme::Light), hex("#F3F2EE"));
    }

    #[test]
    fn every_accent_has_a_unique_non_empty_name() {
        use strum::IntoEnumIterator;
        let mut names: Vec<&str> = Accent::iter().map(Accent::name).collect();
        for name in &names {
            assert!(!name.is_empty());
        }
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Accent::iter().count());
    }
}
