use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

const SANS_REGULAR: &[u8] =
    include_bytes!("../../assets/fonts/IBMPlexSans/IBMPlexSans-Regular.ttf");
const SANS_SEMIBOLD: &[u8] =
    include_bytes!("../../assets/fonts/IBMPlexSans/IBMPlexSans-SemiBold.ttf");
const MONO_REGULAR: &[u8] =
    include_bytes!("../../assets/fonts/IBMPlexMono/IBMPlexMono-Regular.ttf");

const SANS_REGULAR_NAME: &str = "IBMPlexSans-Regular";
const SANS_SEMIBOLD_NAME: &str = "IBMPlexSans-SemiBold";
const MONO_REGULAR_NAME: &str = "IBMPlexMono-Regular";

/// Name of the egui font family used for section headings (semibold, per the design handoff).
pub fn heading_family() -> FontFamily {
    FontFamily::Name(SANS_SEMIBOLD_NAME.into())
}

/// Builds the `FontDefinitions` bundling IBM Plex Sans (UI) and IBM Plex Mono (data/ids/JSON),
/// per the design handoff's type spec.
pub fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();

    fonts.font_data.insert(
        SANS_REGULAR_NAME.to_owned(),
        Arc::new(FontData::from_static(SANS_REGULAR)),
    );
    fonts.font_data.insert(
        SANS_SEMIBOLD_NAME.to_owned(),
        Arc::new(FontData::from_static(SANS_SEMIBOLD)),
    );
    fonts.font_data.insert(
        MONO_REGULAR_NAME.to_owned(),
        Arc::new(FontData::from_static(MONO_REGULAR)),
    );

    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, SANS_REGULAR_NAME.to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, MONO_REGULAR_NAME.to_owned());
    fonts.families.insert(
        heading_family(),
        vec![SANS_SEMIBOLD_NAME.to_owned(), SANS_REGULAR_NAME.to_owned()],
    );

    fonts
}

/// Installs the Gevo font set into the given egui context.
pub fn install_fonts(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_font_bytes_are_non_empty() {
        assert!(!SANS_REGULAR.is_empty());
        assert!(!SANS_SEMIBOLD.is_empty());
        assert!(!MONO_REGULAR.is_empty());
    }

    #[test]
    fn font_data_contains_all_three_fonts() {
        let fonts = font_definitions();
        assert!(fonts.font_data.contains_key(SANS_REGULAR_NAME));
        assert!(fonts.font_data.contains_key(SANS_SEMIBOLD_NAME));
        assert!(fonts.font_data.contains_key(MONO_REGULAR_NAME));
    }

    #[test]
    fn proportional_family_defaults_to_sans_regular() {
        let fonts = font_definitions();
        let first = fonts.families[&FontFamily::Proportional].first();
        assert_eq!(first.map(String::as_str), Some(SANS_REGULAR_NAME));
    }

    #[test]
    fn monospace_family_defaults_to_mono_regular() {
        let fonts = font_definitions();
        let first = fonts.families[&FontFamily::Monospace].first();
        assert_eq!(first.map(String::as_str), Some(MONO_REGULAR_NAME));
    }

    #[test]
    fn heading_family_prefers_semibold_with_regular_fallback() {
        let fonts = font_definitions();
        let heading = &fonts.families[&heading_family()];
        assert_eq!(
            heading.as_slice(),
            &[SANS_SEMIBOLD_NAME.to_owned(), SANS_REGULAR_NAME.to_owned()]
        );
    }

    #[test]
    fn install_fonts_does_not_panic_on_a_fresh_context() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
    }
}
