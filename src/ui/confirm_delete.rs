use eframe::egui::{self, RichText, TextEdit};

/// A pending delete that is only armed once the user types the target's exact name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmDelete {
    target: String,
    detail: String,
    typed: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmOutcome {
    Pending,
    Cancelled,
    Confirmed,
}

impl ConfirmDelete {
    /// `detail` is the consequence line shown under the title, e.g. the document count.
    pub fn new(target: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            detail: detail.into(),
            typed: String::new(),
        }
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    /// True once the typed text matches the target exactly (case-sensitive, no trimming).
    pub fn is_armed(&self) -> bool {
        self.typed == self.target
    }
}

/// Render glue: modal asking to type the target's name. Cancel and Esc dismiss it.
pub fn show(ctx: &egui::Context, state: &mut ConfirmDelete) -> ConfirmOutcome {
    let mut outcome = ConfirmOutcome::Pending;
    let modal = egui::Modal::new(egui::Id::unique("confirm_delete")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.heading(format!("Delete collection {}?", state.target));
        ui.label(&state.detail);
        ui.add_space(8.0);
        ui.label(format!("Type {} to confirm", state.target));
        ui.add(
            TextEdit::singleline(&mut state.typed)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                outcome = ConfirmOutcome::Cancelled;
            }
            let danger = ui.visuals().error_fg_color;
            let delete = ui.add_enabled(
                state.is_armed(),
                egui::Button::new(RichText::new("Delete collection").color(danger)),
            );
            if delete.clicked() {
                outcome = ConfirmOutcome::Confirmed;
            }
        });
    });
    if modal.should_close() && outcome == ConfirmOutcome::Pending {
        outcome = ConfirmOutcome::Cancelled;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(target: &str, text: &str) -> ConfirmDelete {
        let mut c = ConfirmDelete::new(target, "48,210 documents");
        c.typed = text.to_owned();
        c
    }

    #[test]
    fn starts_disarmed() {
        assert!(!ConfirmDelete::new("products", "").is_armed());
    }

    #[test]
    fn exact_name_arms_the_delete() {
        assert!(typed("products", "products").is_armed());
    }

    #[test]
    fn partial_name_does_not_arm() {
        assert!(!typed("products", "product").is_armed());
    }

    #[test]
    fn match_is_case_sensitive() {
        assert!(!typed("products", "Products").is_armed());
    }

    #[test]
    fn whitespace_is_not_trimmed() {
        assert!(!typed("products", "products ").is_armed());
    }

    #[test]
    fn target_is_exposed_for_the_caller() {
        assert_eq!(ConfirmDelete::new("products", "").target(), "products");
    }
}
