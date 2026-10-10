mod domains;
mod infra;
mod services;

use anyhow::Result;
use infra::config::Config;
use infra::telegram_bot::TelegramBot;
use infra::util;
use services::reader::ReaderService;
use services::send::SendService;
use std::env;
use tokio::sync::watch;

/// Upper bound on how long `run` waits for both tasks to wind down after a
/// shutdown signal before the process exits anyway.
const SHUTDOWN_GRACE_SECS: u64 = 10;

fn spawn_signal_watcher(tx: &watch::Sender<bool>) {
    let tx = tx.clone();

    tokio::spawn(async move {
        util::shutdown_signal().await;
        let _ = tx.send(true);
    });
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    let command = match args.get(1) {
        Some(cmd) => cmd.as_str(),
        None => {
            eprintln!("Usage: news-scout <run|read|send|send-test-report>");
            std::process::exit(1);
        }
    };

    let config = Config::load()?;

    let _sentry_guard = if let Some(dsn) = &config.sentry_dsn {
        log::info!("Initializing Sentry...");
        Some(sentry::init((
            dsn.as_str(),
            sentry::ClientOptions {
                release: sentry::release_name!(),
                ..Default::default()
            },
        )))
    } else {
        None
    };

    match command {
        "run" => {
            let (tx, rx) = watch::channel(false);
            spawn_signal_watcher(&tx);

            let cfg = config.clone();
            let reader = tokio::spawn(services::supervisor::supervise(
                "read",
                tx.clone(),
                move |rx| {
                    let cfg = cfg.clone();

                    async move {
                        let mut service = ReaderService::new(cfg).await?;
                        service.run(&mut { rx }).await
                    }
                },
            ));

            let cfg = config.clone();
            let send = tokio::spawn(services::supervisor::supervise(
                "send",
                tx.clone(),
                move |rx| {
                    let cfg = cfg.clone();

                    async move {
                        let service = SendService::new(cfg).await?;
                        service.run(&mut { rx }).await
                    }
                },
            ));

            let joined = async {
                let reader = reader.await;
                let send = send.await;

                match (reader, send) {
                    (Err(e), _) | (_, Err(e)) => {
                        Err(anyhow::anyhow!("supervised task failed: {e}"))
                    }
                    (Ok(Err(e)), _) | (_, Ok(Err(e))) => Err(e),
                    (Ok(Ok(())), Ok(Ok(()))) => Ok(()),
                }
            };

            tokio::pin!(joined);

            let mut watch_rx = rx.clone();
            let outcome = tokio::select! {
                result = &mut joined => Some(result),
                _ = util::wait_shutdown(&mut watch_rx) => None,
            };

            match outcome {
                Some(result) => result?,
                None => {
                    log::info!(
                        "Shutdown signal received; waiting up to {SHUTDOWN_GRACE_SECS}s for tasks to stop."
                    );

                    match tokio::time::timeout(
                        std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS),
                        &mut joined,
                    )
                    .await
                    {
                        Ok(result) => result?,
                        Err(_) => {
                            log::warn!(
                                "Tasks did not stop within {SHUTDOWN_GRACE_SECS}s; exiting anyway."
                            );
                        }
                    }
                }
            }
        }
        "read" => {
            let (tx, rx) = watch::channel(false);
            spawn_signal_watcher(&tx);

            let mut shutdown = rx;
            let mut reader = ReaderService::new(config).await?;
            reader.run(&mut shutdown).await?;
        }
        "send" => {
            let (tx, rx) = watch::channel(false);
            spawn_signal_watcher(&tx);

            let mut shutdown = rx;
            let send = SendService::new(config).await?;
            send.run(&mut shutdown).await?;
        }
        "send-test-report" => {
            let bot = TelegramBot::new(
                config.report_token.clone(),
                config.report_chat_id.chat_id,
                config.report_chat_id.thread_id,
            );

            bot.send_message("This is where forwarded messages will go.")
                .await?;
        }
        _ => {
            eprintln!("Unknown command: {command}");
            eprintln!("Usage: news-scout [run|read|send|send-test-report]");
            std::process::exit(1);
        }
    }

    Ok(())
}
