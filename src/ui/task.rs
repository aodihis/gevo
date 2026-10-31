use std::future::Future;
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui;
use tokio::runtime::{Builder, Runtime};

/// One runtime for every request, so a client's pooled connections outlive any single task.
fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// Runs `fut` on the shared runtime and wakes the UI when the result is ready.
/// Dropping the receiver discards the result.
pub fn run<T, Fut>(ctx: egui::Context, fut: Fut) -> Receiver<T>
where
    T: Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
{
    let (tx, rx) = channel();
    runtime().spawn(async move {
        let _ = tx.send(fut.await);
        ctx.request_repaint();
    });
    rx
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn delivers_the_future_output() {
        let rx = run(egui::Context::default(), async { 5 });
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(5));
    }

    #[test]
    fn supports_timers_and_spawn_blocking() {
        let rx = run(egui::Context::default(), async {
            tokio::time::sleep(Duration::from_millis(5)).await;
            tokio::task::spawn_blocking(|| "done").await.unwrap()
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok("done"));
    }

    #[test]
    fn tasks_share_one_runtime() {
        let id = || tokio::runtime::Handle::current().id();
        let a = run(egui::Context::default(), async move { id() });
        let b = run(egui::Context::default(), async move { id() });
        let wait = Duration::from_secs(5);
        assert_eq!(a.recv_timeout(wait), b.recv_timeout(wait));
    }
}
