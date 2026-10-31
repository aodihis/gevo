use eframe::egui::{self, RichText, TextEdit};

/// A pending destructive action. A typed confirmation is armed only once the user types the
/// target's exact name; a plain one is armed from the start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmDelete {
    target: String,
    title: String,
    detail: String,
    button: String,
    requires_typing: bool,
    typed: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmOutcome {
    Pending,
    Cancelled,
    Confirmed,
}

impl ConfirmDelete {
    /// Deleting a collection: the name must be typed. `detail` is the consequence line.
    pub fn new(target: impl Into<String>, detail: impl Into<String>) -> Self {
        let target = target.into();
        Self {
            title: format!("Delete collection {target}?"),
            button: "Delete collection".to_owned(),
            target,
            detail: detail.into(),
            requires_typing: true,
            typed: String::new(),
        }
    }

    /// A confirm/cancel pair for lower-stakes actions such as deleting one document.
    pub fn plain(
        target: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
        button: impl Into<String>,
    ) -> Self {
        Self {
            target: target.into(),
            title: title.into(),
            detail: detail.into(),
            button: button.into(),
            requires_typing: false,
            typed: String::new(),
        }
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    /// True once the action may be confirmed. Typed matching is case-sensitive and untrimmed.
    pub fn is_armed(&self) -> bool {
        !self.requires_typing || self.typed == self.target
    }
}

/// Render glue: the confirmation modal. Cancel and Esc dismiss it.
pub fn show(ctx: &egui::Context, state: &mut ConfirmDelete) -> ConfirmOutcome {
    let mut outcome = ConfirmOutcome::Pending;
    let modal = egui::Modal::new(egui::Id::unique("confirm_delete")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.heading(&state.title);
        ui.label(&state.detail);
        if state.requires_typing {
            ui.add_space(8.0);
            ui.label(format!("Type {} to confirm", state.target));
            ui.add(
                TextEdit::singleline(&mut state.typed)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                outcome = ConfirmOutcome::Cancelled;
            }
            let danger = ui.visuals().error_fg_color;
            let confirm = ui.add_enabled(
                state.is_armed(),
                egui::Button::new(RichText::new(&state.button).color(danger)),
            );
            if confirm.clicked() {
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

    #[test]
    fn a_collection_delete_names_the_collection_in_its_title() {
        let c = ConfirmDelete::new("products", "");
        assert_eq!(c.title, "Delete collection products?");
        assert_eq!(c.button, "Delete collection");
    }

    #[test]
    fn a_plain_confirmation_is_armed_without_typing() {
        let c = ConfirmDelete::plain(
            "sku-1",
            "Delete document sku-1?",
            "Cannot be undone.",
            "Delete",
        );
        assert!(c.is_armed());
        assert_eq!(c.target(), "sku-1");
        assert_eq!(c.button, "Delete");
    }
}
