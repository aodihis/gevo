use std::future::Future;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui;

/// Runs `fut` to completion on a worker thread and wakes the UI when the result is ready.
/// Dropping the receiver discards the result.
pub fn run<T, Fut>(ctx: egui::Context, fut: Fut) -> Receiver<T>
where
    T: Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
{
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let _ = tx.send(runtime.block_on(fut));
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
}
