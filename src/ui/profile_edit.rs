use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, ComboBox, RichText, TextEdit};
use zeroize::Zeroizing;

use crate::domain::connection_service::{
    ConnectionService, ConnectionTestReport, SdkClientFactory,
};
use crate::domain::credential_store::KeyringCredentialStore;
use crate::domain::profile::ConnectionProfile;
use crate::domain::profile_store::FileProfileStore;
use crate::domain::russh_tunnel::RusshTunnel;
use crate::domain::ssh_tunnel::HostKeyPrompt;
use crate::ui::profile_form::{
    Field, FieldError, ProfileForm, Protocol, RowState, SshAuthKind, TestProgress, WizardStep,
    record_save, run_test, save, wizard_rows,
};

/// What the screen asks the app to do after a frame.
pub enum Outcome {
    None,
    Cancelled,
    Saved,
}

/// State of the connection edit screen. Render glue — exempt from unit tests per `AGENTS.md`
/// §6; the logic lives in `profile_form`.
pub struct ProfileEditScreen {
    form: ProfileForm,
    errors: Vec<FieldError>,
    save_error: Option<String>,
    progress: TestProgress,
    pending: Option<Receiver<ConnectionTestReport>>,
    store: FileProfileStore,
    creds: KeyringCredentialStore,
    prompt: Arc<dyn HostKeyPrompt>,
}

impl ProfileEditScreen {
    pub fn new(existing: Option<&ConnectionProfile>, prompt: Arc<dyn HostKeyPrompt>) -> Self {
        let creds = KeyringCredentialStore::new();
        let form = match existing {
            Some(profile) => ProfileForm::load(profile, &creds),
            None => ProfileForm::blank(),
        };
        Self {
            form,
            errors: Vec::new(),
            save_error: None,
            progress: TestProgress::Idle,
            pending: None,
            store: FileProfileStore::new(FileProfileStore::default_path()),
            creds,
            prompt,
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui) -> Outcome {
        self.poll_test(ui.ctx());
        let mut outcome = Outcome::None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Server");
            self.server_section(ui);
            ui.add_space(12.0);
            ui.checkbox(&mut self.form.ssh_enabled, "SSH tunnel");
            if self.form.ssh_enabled {
                self.ssh_section(ui);
            }
            ui.add_space(12.0);
            self.test_card(ui);
            ui.add_space(12.0);
            if let Some(msg) = &self.save_error {
                ui.colored_label(ui.visuals().error_fg_color, msg);
            }
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancelled;
                }
                if ui
                    .add_enabled(self.pending.is_none(), egui::Button::new("Test connection"))
                    .clicked()
                {
                    self.start_test(ui.ctx());
                }
                if ui.button("Save").clicked() && self.try_save() {
                    outcome = Outcome::Saved;
                }
            });
        });
        outcome
    }

    fn server_section(&mut self, ui: &mut egui::Ui) {
        let form = &mut self.form;
        let errors = &self.errors;
        text_row(ui, "Display name", &mut form.name, Field::Name, errors);
        ui.horizontal(|ui| {
            ui.label("Protocol");
            ComboBox::from_id_salt("protocol")
                .selected_text(form.protocol.scheme())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut form.protocol, Protocol::Http, "http");
                    ui.selectable_value(&mut form.protocol, Protocol::Https, "https");
                });
        });
        text_row(ui, "Host", &mut form.host, Field::Host, errors);
        text_row(ui, "Port", &mut form.port, Field::Port, errors);
        secret_row(
            ui,
            "Admin API key",
            &mut form.admin_key,
            Field::AdminKey,
            errors,
        );
        ui.weak("Stored in your OS keychain, never in the profiles file.");
    }

    fn ssh_section(&mut self, ui: &mut egui::Ui) {
        let form = &mut self.form;
        let errors = &self.errors;
        text_row(ui, "SSH host", &mut form.ssh_host, Field::SshHost, errors);
        text_row(ui, "SSH port", &mut form.ssh_port, Field::SshPort, errors);
        text_row(
            ui,
            "Username",
            &mut form.ssh_username,
            Field::SshUsername,
            errors,
        );
        ui.horizontal(|ui| {
            ui.label("Authentication");
            let before = form.ssh_auth;
            ui.selectable_value(&mut form.ssh_auth, SshAuthKind::Password, "Password");
            ui.selectable_value(&mut form.ssh_auth, SshAuthKind::PrivateKey, "Private key");
            if form.ssh_auth != before {
                form.ssh_secret = Zeroizing::default();
            }
        });
        if form.ssh_auth == SshAuthKind::PrivateKey {
            text_row(
                ui,
                "Private key file",
                &mut form.key_path,
                Field::KeyPath,
                errors,
            );
        }
        let label = match form.ssh_auth {
            SshAuthKind::Password => "Password",
            SshAuthKind::PrivateKey => "Passphrase (optional)",
        };
        secret_row(ui, label, &mut form.ssh_secret, Field::SshHost, &[]);
        text_row(
            ui,
            "Local bind port",
            &mut form.local_bind_port,
            Field::LocalBindPort,
            errors,
        );
    }

    fn test_card(&self, ui: &mut egui::Ui) {
        ui.heading("Connection test");
        let rows = wizard_rows(&self.progress);
        for (step, state) in WizardStep::ALL.into_iter().zip(&rows) {
            ui.horizontal(|ui| {
                let (mark, hint) = match state {
                    RowState::Pending => ("·", String::new()),
                    RowState::Running => ("…", String::new()),
                    RowState::Ok => ("OK", String::new()),
                    RowState::Skipped => ("–", "not applicable".into()),
                    RowState::NotRun => ("–", "not run".into()),
                    RowState::Failed(e) => ("✗", format!("{}: {e}", e.category())),
                };
                ui.label(mark);
                ui.label(step.label());
                if !hint.is_empty() {
                    let text = RichText::new(hint);
                    if matches!(state, RowState::Failed(_)) {
                        ui.label(text.color(ui.visuals().error_fg_color));
                    } else {
                        ui.label(text.weak());
                    }
                }
            });
        }
    }

    fn start_test(&mut self, ctx: &egui::Context) {
        self.errors = self.form.validate();
        if !self.errors.is_empty() {
            return;
        }
        let (tx, rx) = channel();
        self.pending = Some(rx);
        self.progress = TestProgress::Running;
        // The form is read-only during the test, so a snapshot of the secrets is enough.
        let form = self.form.clone();
        let ctx = ctx.clone();
        let prompt = Arc::clone(&self.prompt);
        std::thread::spawn(move || {
            let service = ConnectionService::new(RusshTunnel::new(prompt), SdkClientFactory);
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            if let Ok(report) = runtime.block_on(run_test(&service, &form)) {
                let _ = tx.send(report);
            }
            ctx.request_repaint();
        });
    }

    fn poll_test(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.pending else { return };
        match rx.try_recv() {
            Ok(report) => {
                self.progress = TestProgress::Done(report);
                self.pending = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.progress = TestProgress::Idle;
                self.pending = None;
            }
        }
    }

    fn try_save(&mut self) -> bool {
        let result = save(&self.form, &self.store, &self.creds);
        record_save(result, &mut self.errors, &mut self.save_error)
    }
}

fn text_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    field: Field,
    errors: &[FieldError],
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(TextEdit::singleline(value));
        error_for(ui, field, errors);
    });
}

fn secret_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Zeroizing<String>,
    field: Field,
    errors: &[FieldError],
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(TextEdit::singleline(&mut **value).password(true));
        error_for(ui, field, errors);
    });
}

fn error_for(ui: &mut egui::Ui, field: Field, errors: &[FieldError]) {
    if let Some(e) = errors.iter().find(|e| e.field == field) {
        ui.colored_label(ui.visuals().error_fg_color, e.message);
    }
}
