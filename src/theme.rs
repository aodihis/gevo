use eframe::egui::{Color32, Stroke, Visuals};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    Amber,
    Blue,
    Green,
    Violet,
}

struct Tokens {
    page_bg: Color32,
    panel_bg: Color32,
    raised: Color32,
    selected: Color32,
    border: Color32,
    input_border: Color32,
    text: Color32,
    text_muted: Color32,
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

fn accent_hex(accent: Accent, mode: ThemeMode) -> &'static str {
    match (accent, mode) {
        (Accent::Amber, ThemeMode::Dark) => "#E8A33D",
        (Accent::Amber, ThemeMode::Light) => "#A8650F",
        (Accent::Blue, ThemeMode::Dark) => "#6EA8FE",
        (Accent::Blue, ThemeMode::Light) => "#2F64C8",
        (Accent::Green, ThemeMode::Dark) => "#5FC08A",
        (Accent::Green, ThemeMode::Light) => "#23804D",
        (Accent::Violet, ThemeMode::Dark) => "#B394F5",
        (Accent::Violet, ThemeMode::Light) => "#6C46C9",
    }
}

fn tokens(mode: ThemeMode, accent: Accent) -> Tokens {
    let accent_fill = hex(accent_hex(accent, mode));
    let accent_text = match mode {
        ThemeMode::Dark => hex("#1A1408"),
        ThemeMode::Light => hex("#FFFFFF"),
    };
    match mode {
        ThemeMode::Dark => Tokens {
            page_bg: hex("#121315"),
            panel_bg: hex("#17181B"),
            raised: hex("#202226"),
            selected: hex("#26282D"),
            border: hex("#2C2E33"),
            input_border: hex("#34363C"),
            text: hex("#E7E5E0"),
            text_muted: hex("#9A9890"),
            danger_text: hex("#F59A85"),
            accent_fill,
            accent_text,
        },
        ThemeMode::Light => Tokens {
            page_bg: hex("#FBFAF8"),
            panel_bg: hex("#FFFFFF"),
            raised: hex("#F3F2EE"),
            selected: hex("#E4E2DB"),
            border: hex("#DDDBD4"),
            input_border: hex("#D3D1CA"),
            text: hex("#1D1D1B"),
            text_muted: hex("#67655F"),
            danger_text: hex("#7A4B05"),
            accent_fill,
            accent_text,
        },
    }
}

/// Builds an `egui::Visuals` from the Gevo design tokens for the given mode + accent.
pub fn build_visuals(mode: ThemeMode, accent: Accent) -> Visuals {
    let t = tokens(mode, accent);
    let mut visuals = match mode {
        ThemeMode::Dark => Visuals::dark(),
        ThemeMode::Light => Visuals::light(),
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
    visuals.faint_bg_color = t.text_muted;

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
        let v = build_visuals(ThemeMode::Dark, Accent::Amber);
        assert_eq!(v.window_fill, hex("#121315"));
        assert_eq!(v.panel_fill, hex("#17181B"));
        assert_eq!(v.override_text_color, Some(hex("#E7E5E0")));
    }

    #[test]
    fn light_mode_uses_light_page_and_panel_tokens() {
        let v = build_visuals(ThemeMode::Light, Accent::Amber);
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
            let v = build_visuals(ThemeMode::Dark, accent);
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
            let v = build_visuals(ThemeMode::Light, accent);
            assert_eq!(v.selection.bg_fill, hex(expected), "{accent:?}");
        }
    }

    #[test]
    fn accent_text_is_dark_in_dark_mode_and_white_in_light_mode() {
        let dark = build_visuals(ThemeMode::Dark, Accent::Blue);
        let light = build_visuals(ThemeMode::Light, Accent::Blue);
        assert_eq!(dark.selection.stroke.color, hex("#1A1408"));
        assert_eq!(light.selection.stroke.color, hex("#FFFFFF"));
    }

    #[test]
    fn danger_text_differs_between_modes() {
        let dark = build_visuals(ThemeMode::Dark, Accent::Amber);
        let light = build_visuals(ThemeMode::Light, Accent::Amber);
        assert_eq!(dark.error_fg_color, hex("#F59A85"));
        assert_eq!(light.error_fg_color, hex("#7A4B05"));
    }
}
