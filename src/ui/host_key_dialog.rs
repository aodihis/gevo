use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use eframe::egui;

use crate::domain::ssh_tunnel::HostKeyPrompt;

/// How long a worker waits for the user before treating silence as a refusal.
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);

/// A host key awaiting a trust decision; `answer` unblocks the waiting worker.
pub struct HostKeyRequest {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    reply: Sender<bool>,
    abandoned: Arc<AtomicBool>,
}

impl HostKeyRequest {
    pub fn answer(self, trust: bool) {
        let _ = self.reply.send(trust);
    }
}

/// Worker-side prompt: forwards the question to the UI thread and blocks for the answer.
pub struct ChannelHostKeyPrompt {
    requests: Sender<HostKeyRequest>,
    timeout: Duration,
}

impl HostKeyPrompt for ChannelHostKeyPrompt {
    fn confirm(&self, host: &str, port: u16, key_type: &str, fingerprint_sha256: &str) -> bool {
        let (reply, answer) = channel();
        let abandoned = Arc::new(AtomicBool::new(false));
        let request = HostKeyRequest {
            host: host.to_owned(),
            port,
            key_type: key_type.to_owned(),
            fingerprint: fingerprint_sha256.to_owned(),
            reply,
            abandoned: Arc::clone(&abandoned),
        };
        if self.requests.send(request).is_err() {
            return false;
        }
        let trusted = answer.recv_timeout(self.timeout).unwrap_or(false);
        // Lets the inbox discard a request nobody is waiting on any more.
        abandoned.store(true, Ordering::Release);
        trusted
    }
}

/// UI-side queue of pending requests; the front one is shown in the dialog.
pub struct HostKeyInbox {
    incoming: Receiver<HostKeyRequest>,
    queue: VecDeque<HostKeyRequest>,
}

pub fn host_key_channel(timeout: Duration) -> (ChannelHostKeyPrompt, HostKeyInbox) {
    let (requests, incoming) = channel();
    (
        ChannelHostKeyPrompt { requests, timeout },
        HostKeyInbox {
            incoming,
            queue: VecDeque::new(),
        },
    )
}

impl HostKeyInbox {
    /// Collects new requests and returns the one to show, if any.
    pub fn current(&mut self) -> Option<&HostKeyRequest> {
        self.sync();
        self.queue.front()
    }

    fn sync(&mut self) {
        self.queue.extend(self.incoming.try_iter());
        self.queue.retain(|r| !r.abandoned.load(Ordering::Acquire));
    }

    /// Answers the request being shown; no-op when none is pending.
    pub fn resolve(&mut self, trust: bool) {
        self.sync();
        if let Some(request) = self.queue.pop_front() {
            request.answer(trust);
        }
    }
}

/// Render glue: modal asking whether to trust an unknown host key. Cancel is the default.
pub fn show(ctx: &egui::Context, inbox: &mut HostKeyInbox) {
    let Some(request) = inbox.current() else {
        return;
    };
    let mut decision = None;
    let modal = egui::Modal::new(egui::Id::unique("host_key_dialog")).show(ctx, |ui| {
        ui.heading("Unknown SSH host");
        ui.label(format!("Host: {}:{}", request.host, request.port));
        ui.label(format!("Key type: {}", request.key_type));
        ui.label("SHA-256 fingerprint:");
        ui.monospace(&request.fingerprint);
        ui.label("Verify this fingerprint with the server's owner before trusting it.");
        ui.horizontal(|ui| {
            if ui.button("Trust and connect").clicked() {
                decision = Some(true);
            }
            let cancel = ui.button("Cancel");
            cancel.request_focus();
            if cancel.clicked() {
                decision = Some(false);
            }
        });
    });
    if modal.should_close() {
        decision.get_or_insert(false);
    }
    if let Some(trust) = decision {
        inbox.resolve(trust);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(20);
    const LONG: Duration = Duration::from_secs(5);

    fn ask(
        prompt: std::sync::Arc<ChannelHostKeyPrompt>,
        host: &'static str,
    ) -> std::thread::JoinHandle<bool> {
        std::thread::spawn(move || prompt.confirm(host, 22, "ssh-ed25519", "SHA256:abc"))
    }

    fn wait_until(inbox: &mut HostKeyInbox, queued: usize) {
        while {
            inbox.current();
            inbox.queue.len() < queued
        } {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn trust_answer_reaches_the_worker() {
        let (prompt, mut inbox) = host_key_channel(LONG);
        let worker = ask(prompt.into(), "example.com");
        wait_until(&mut inbox, 1);
        let req = inbox.current().unwrap();
        assert_eq!(
            (req.host.as_str(), req.port, req.key_type.as_str()),
            ("example.com", 22, "ssh-ed25519")
        );
        assert_eq!(req.fingerprint, "SHA256:abc");
        inbox.resolve(true);
        assert!(worker.join().unwrap());
    }

    #[test]
    fn cancel_answer_reaches_the_worker_as_false() {
        let (prompt, mut inbox) = host_key_channel(LONG);
        let worker = ask(prompt.into(), "h");
        wait_until(&mut inbox, 1);
        inbox.resolve(false);
        assert!(!worker.join().unwrap());
    }

    #[test]
    fn unanswered_request_times_out_to_false() {
        let (prompt, _inbox) = host_key_channel(MS);
        assert!(!prompt.confirm("h", 22, "k", "f"));
    }

    #[test]
    fn timed_out_request_is_not_shown() {
        let (prompt, mut inbox) = host_key_channel(MS);
        assert!(!prompt.confirm("h", 22, "k", "f"));
        assert!(inbox.current().is_none());
    }

    #[test]
    fn answering_an_abandoned_request_does_not_panic() {
        let (prompt, mut inbox) = host_key_channel(MS);
        assert!(!prompt.confirm("h", 22, "k", "f"));
        inbox.resolve(true);
        assert!(inbox.current().is_none());
    }

    #[test]
    fn dropped_inbox_means_false() {
        let (prompt, inbox) = host_key_channel(LONG);
        drop(inbox);
        assert!(!prompt.confirm("h", 22, "k", "f"));
    }

    #[test]
    fn request_dropped_without_answer_means_false() {
        let (prompt, mut inbox) = host_key_channel(LONG);
        let worker = ask(prompt.into(), "h");
        wait_until(&mut inbox, 1);
        inbox.queue.clear();
        assert!(!worker.join().unwrap());
    }

    #[test]
    fn requests_are_served_in_order() {
        let (prompt, mut inbox) = host_key_channel(LONG);
        let prompt = std::sync::Arc::new(prompt);
        let first = ask(prompt.clone(), "a");
        wait_until(&mut inbox, 1);
        let second = ask(prompt, "b");
        wait_until(&mut inbox, 2);
        assert_eq!(inbox.current().unwrap().host, "a");
        inbox.resolve(true);
        assert!(first.join().unwrap());
        assert_eq!(inbox.current().unwrap().host, "b");
        inbox.resolve(false);
        assert!(!second.join().unwrap());
    }

    #[test]
    fn resolve_with_nothing_pending_is_a_no_op() {
        let (_prompt, mut inbox) = host_key_channel(MS);
        inbox.resolve(true);
        assert!(inbox.current().is_none());
    }
}
