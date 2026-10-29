use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Theme};

use crate::domain::error::GevoError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipKind {
    Ok,
    Network,
    Tunnel,
    Auth,
    Api,
}

/// What a chip shows: a category label and, for errors, the verbatim message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChipView<'a> {
    pub kind: ChipKind,
    pub label: &'static str,
    pub detail: Option<&'a str>,
}

impl<'a> ChipView<'a> {
    pub fn ok() -> Self {
        Self {
            kind: ChipKind::Ok,
            label: "OK",
            detail: None,
        }
    }

    pub fn from_error(err: &'a GevoError) -> Self {
        let (kind, detail) = match err {
            GevoError::Network(m) => (ChipKind::Network, m.as_str()),
            GevoError::Tunnel(m) => (ChipKind::Tunnel, m.as_str()),
            GevoError::Auth(m) => (ChipKind::Auth, m.as_str()),
            GevoError::Api(m) => (ChipKind::Api, m.0.as_str()),
        };
        Self {
            kind,
            label: err.category(),
            detail: Some(detail),
        }
    }
}

fn hex(s: &str) -> Color32 {
    let n = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap();
    Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8)
}

/// (background, foreground) from the handoff's status chip tables.
/// The handoff defines no API chip; it uses the danger palette.
pub fn chip_colors(kind: ChipKind, mode: Theme) -> (Color32, Color32) {
    let (bg, fg) = match (kind, mode) {
        (ChipKind::Ok, Theme::Dark) => ("#1C3326", "#8FE0B0"),
        (ChipKind::Auth, Theme::Dark) => ("#3A2C14", "#F7C978"),
        (ChipKind::Tunnel, Theme::Dark) => ("#2D2340", "#CDB2F5"),
        (ChipKind::Network, Theme::Dark) => ("#1B2A40", "#A9C8F7"),
        (ChipKind::Api, Theme::Dark) => ("#3D1F19", "#F59A85"),
        (ChipKind::Ok, Theme::Light) => ("#DDF1E4", "#1B6B3F"),
        (ChipKind::Auth, Theme::Light) => ("#F8E7C6", "#7A4B05"),
        (ChipKind::Tunnel, Theme::Light) => ("#ECE2FA", "#5B3AA8"),
        (ChipKind::Network, Theme::Light) => ("#DDE8FA", "#2552A0"),
        (ChipKind::Api, Theme::Light) => ("#F7DDD7", "#B8402C"),
    };
    (hex(bg), hex(fg))
}

/// Renders the pill; the detail message, if any, is shown on hover.
pub fn status_chip(ui: &mut egui::Ui, view: &ChipView) -> egui::Response {
    let (bg, fg) = chip_colors(view.kind, ui.ctx().theme());
    let response = Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(view.label).color(fg).size(11.0).strong());
        })
        .response;
    match view.detail {
        Some(detail) => response.on_hover_text(detail),
        None => response,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::error::TypesenseMsg;

    #[test]
    fn from_error_maps_each_category() {
        let cases = [
            (GevoError::Network("n".into()), ChipKind::Network, "NETWORK"),
            (GevoError::Tunnel("t".into()), ChipKind::Tunnel, "TUNNEL"),
            (GevoError::Auth("a".into()), ChipKind::Auth, "AUTH"),
            (
                GevoError::Api(TypesenseMsg("p".into())),
                ChipKind::Api,
                "API",
            ),
        ];
        for (err, kind, label) in &cases {
            let v = ChipView::from_error(err);
            assert_eq!((v.kind, v.label), (*kind, *label));
        }
    }

    #[test]
    fn from_error_carries_message_verbatim() {
        let err = GevoError::Api(TypesenseMsg("Not found.".into()));
        assert_eq!(ChipView::from_error(&err).detail, Some("Not found."));
        let err = GevoError::Auth("bad key".into());
        assert_eq!(ChipView::from_error(&err).detail, Some("bad key"));
    }

    #[test]
    fn ok_chip_has_no_detail() {
        let v = ChipView::ok();
        assert_eq!((v.kind, v.label, v.detail), (ChipKind::Ok, "OK", None));
    }

    #[test]
    fn colors_match_handoff_tables() {
        assert_eq!(
            chip_colors(ChipKind::Network, Theme::Dark),
            (hex("#1B2A40"), hex("#A9C8F7"))
        );
        assert_eq!(
            chip_colors(ChipKind::Tunnel, Theme::Light),
            (hex("#ECE2FA"), hex("#5B3AA8"))
        );
        assert_eq!(
            chip_colors(ChipKind::Auth, Theme::Dark),
            (hex("#3A2C14"), hex("#F7C978"))
        );
        assert_eq!(
            chip_colors(ChipKind::Ok, Theme::Light),
            (hex("#DDF1E4"), hex("#1B6B3F"))
        );
    }

    #[test]
    fn api_chip_uses_danger_text_and_every_kind_differs() {
        assert_eq!(chip_colors(ChipKind::Api, Theme::Dark).1, hex("#F59A85"));
        let kinds = [
            ChipKind::Ok,
            ChipKind::Network,
            ChipKind::Tunnel,
            ChipKind::Auth,
            ChipKind::Api,
        ];
        for mode in [Theme::Dark, Theme::Light] {
            for (i, a) in kinds.iter().enumerate() {
                for b in &kinds[i + 1..] {
                    assert_ne!(chip_colors(*a, mode), chip_colors(*b, mode));
                }
            }
        }
    }
}
