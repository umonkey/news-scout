use tokio::sync::watch;
use tokio::time::{Duration, sleep as tokio_sleep};

/// Shared "shutdown requested" flag, updated by the single signal watcher in
/// `main` and observed by the running services.
pub type ShutdownRx = watch::Receiver<bool>;

pub async fn sleep(seconds: u64) {
    log::debug!("Waiting {seconds} seconds...");
    tokio_sleep(Duration::from_secs(seconds)).await;
}

/// Resolves immediately if shutdown was already requested, otherwise waits
/// until it is.
pub async fn wait_shutdown(rx: &mut ShutdownRx) {
    if *rx.borrow_and_update() {
        return;
    }

    let _ = rx.changed().await;
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
