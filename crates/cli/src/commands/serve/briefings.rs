//! serve's pass over the picked-up briefings: once when it starts, then
//! every [`EVERY`], it closes as expired those picked up longer ago than the
//! config file's `briefing-close-after`, and logs each (AG-19).

use std::sync::Arc;
use std::time::Duration;

use cases_core::{Append, Store as _};
use tokio::sync::watch as channel;
use tokio::time::MissedTickBehavior;

use super::app::App;
use crate::{briefings, output};

/// How often serve looks for briefings to close.
const EVERY: Duration = Duration::from_hours(1);

/// Closes the briefings picked up at least `after` ago, at once and then
/// every [`EVERY`], until serve stops.
pub(super) async fn close_stale(app: Arc<App>, after: Duration) {
    let stopping = app.stopping.clone();
    every(EVERY, stopping, move || pass(&app, after)).await;
}

/// Runs `pass` on a blocking thread at once, then every `interval`, until
/// `stopping` changes. A pass that takes longer than `interval` is not
/// followed by the ones it missed.
async fn every(
    interval: Duration,
    mut stopping: channel::Receiver<bool>,
    pass: impl Fn() + Send + Sync + 'static,
) {
    let pass = Arc::new(pass);
    let mut ticks = tokio::time::interval(interval);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticks.tick() => {}
            _ = stopping.changed() => return,
        }
        let pass = Arc::clone(&pass);
        let _ = tokio::task::spawn_blocking(move || pass()).await;
    }
}

/// One pass: reads the store, closes as expired each briefing picked up at
/// least `after` ago, and logs each close, or why it was not made.
fn pass(app: &App, after: Duration) {
    let snapshot = match app.watch.read() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            app.log.line(format!(
                "could not read the store for old briefings: {error}"
            ));
            return;
        }
    };
    let Ok(now) = output::now() else { return };
    let stale = briefings::stale(&snapshot.cases, after, now);
    let writer = briefings::writer();
    let written = briefings::close_each(&stale, after, |key, record, revision| {
        let append = Append::new(briefings::AUTHOR, record);
        app.db.append(&key, &append, &writer, Some(revision))
    });
    let reason = briefings::reason(after);
    for (key, written) in written {
        app.log.line(match written {
            Ok(_) => format!("closed briefing {key}: {reason}"),
            Err(error) => format!("could not close briefing {key}: {error}"),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// The pass runs at once, then each interval, and not once serve stops.
    #[tokio::test(start_paused = true)]
    async fn the_pass_runs_at_once_then_each_interval_until_serve_stops() {
        let runs = Arc::new(AtomicU32::new(0));
        let (stop, stopping) = channel::channel(false);
        let counted = Arc::clone(&runs);
        let task = tokio::spawn(every(Duration::from_secs(10), stopping, move || {
            counted.fetch_add(1, Ordering::SeqCst);
        }));
        let at = |ms| tokio::time::sleep(Duration::from_millis(ms));
        at(1).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        at(10_000).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        at(20_000).await;
        assert_eq!(runs.load(Ordering::SeqCst), 4);
        stop.send_replace(true);
        task.await.unwrap();
        at(30_000).await;
        assert_eq!(runs.load(Ordering::SeqCst), 4);
    }
}
