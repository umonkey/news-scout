use crate::infra::util::{self, ShutdownRx};
use anyhow::Result;
use futures::FutureExt;
use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::time::Duration;
use tokio::sync::watch;

/// Starting delay of the per-task restart backoff.
const BACKOFF_START: Duration = Duration::from_secs(1);
/// Maximum delay between restart attempts.
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// A run that stayed up this long is considered healthy, so the backoff resets.
const BACKOFF_RESET_AFTER: Duration = Duration::from_secs(60);

/// Marker error for failures that must not be retried in place.
///
/// When a task returns one, the supervisor stops that task, requests shutdown
/// of the other, and propagates the error so `main` exits non-zero (letting
/// Docker's `restart: unless-stopped` restart the whole container).
#[derive(Debug)]
pub struct FatalError(pub anyhow::Error);

impl std::fmt::Display for FatalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for FatalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

struct Backoff {
    current: Duration,
}

impl Backoff {
    fn new() -> Self {
        Self {
            current: BACKOFF_START,
        }
    }

    fn reset(&mut self) {
        self.current = BACKOFF_START;
    }

    fn next(&mut self) -> Duration {
        let delay = self.current;
        self.current = (self.current * 2).min(BACKOFF_MAX);
        delay
    }
}

/// Runs `make` repeatedly until shutdown is requested or the task fails fatally.
///
/// A crash (error return or panic) is restarted in place with exponential
/// backoff, without disturbing the other supervised task. The backoff wait is
/// itself shutdown-aware so Ctrl-C never blocks on it.
pub async fn supervise<F, Fut>(
    name: &'static str,
    shutdown: watch::Sender<bool>,
    mut make: F,
) -> Result<()>
where
    F: FnMut(ShutdownRx) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let mut backoff = Backoff::new();

    loop {
        if *shutdown.borrow() {
            return Ok(());
        }

        let started = tokio::time::Instant::now();
        let outcome = AssertUnwindSafe(make(shutdown.subscribe()))
            .catch_unwind()
            .await;

        if started.elapsed() >= BACKOFF_RESET_AFTER {
            backoff.reset();
        }

        match outcome {
            Ok(Ok(())) => {
                if *shutdown.borrow() {
                    return Ok(());
                }

                log::warn!("{name} task exited unexpectedly; restarting.");
            }
            Ok(Err(e)) if e.downcast_ref::<FatalError>().is_some() => {
                log::error!("{name} task hit a fatal error: {e}");
                sentry::capture_error(&*e);

                let _ = shutdown.send(true);

                return Err(e);
            }
            Ok(Err(e)) => {
                log::error!("{name} task failed: {e}");
                sentry::capture_error(&*e);
            }
            Err(panic) => {
                let message = panic_message(&*panic);
                log::error!("{name} task panicked: {message}");
                sentry::capture_error(&*anyhow::anyhow!("{name} task panicked: {message}"));
            }
        }

        let delay = backoff.next();
        log::info!("Restarting {name} task in {}s.", delay.as_secs());

        let mut rx = shutdown.subscribe();

        tokio::select! {
            _ = util::sleep(delay.as_secs()) => {}
            _ = util::wait_shutdown(&mut rx) => return Ok(()),
        }
    }
}

fn panic_message(panic: &(dyn Any + Send)) -> &str {
    if let Some(message) = panic.downcast_ref::<&str>() {
        message
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.as_str()
    } else {
        "unknown panic"
    }
}
