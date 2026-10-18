//! Live data for the panel's pages, as server-sent events: one producer per
//! subject shared by every viewer (the framework's hub), and the shaping of
//! log lines into events.
//!
//! Hub keys name the data, never a user: `server`, `container-stats:<id>`.
//! Who may see a container is checked by each stream's `target`.

use std::sync::Arc;

use rustcms::realtime::{Event, Hub, HubLimits, Shutdown, StreamExt, Subscription, stream};
use rustcms::{AppError, AppResult};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use crate::services::docker_service::{Docker, LogLine, Scope};
use crate::services::system_stats_service::SystemStats;

/// JSON bytes of log lines in one event (under the framework's 64 KiB).
pub const LOG_EVENT_BYTES: usize = 48 * 1024;
/// One log line is cut until its JSON fits in this many bytes.
pub const LOG_LINE_BYTES: usize = 16 * 1024;
/// Lines read at once before they are sent.
const LOG_BATCH_LINES: usize = 500;
/// Log events leave at most this fast (bytes per second), so a long tail or
/// a burst never fills the framework's per-stream queue (1 MiB) faster than a
/// client reads it: that would drop the stream (`lagging`).
const LOG_BYTES_PER_SECOND: usize = 2 * 1024 * 1024;

/// The service the app binds (`app.service::<Live>()`).
pub struct Live {
    hub: Hub<String>,
}

impl Live {
    pub fn new(shutdown: Arc<Shutdown>) -> Self {
        Self {
            hub: Hub::new(shutdown, HubLimits::default()),
        }
    }

    /// Every new host sample (`sample` events).
    pub fn server(&self, stats: SystemStats) -> AppResult<Subscription<String>> {
        self.subscribe("server".to_owned(), move || {
            stream::unfold(stats.subscribe(), |mut samples| async move {
                let event = match samples.recv().await {
                    Ok(sample) => Event::json("sample", &sample).ok(),
                    // Samples were missed here, for every viewer: they read the history again.
                    Err(RecvError::Lagged(_)) => Event::json("resync", &json!({})).ok(),
                    Err(RecvError::Closed) => return None,
                };
                Some((event, samples))
            })
            .filter_map(|event| async move { event })
        })
    }

    /// The container's resources (`stats` events) while it runs. `id` is
    /// the full id its viewers were authorized for.
    pub fn container_stats(&self, docker: Docker, id: String) -> AppResult<Subscription<String>> {
        self.subscribe(format!("container-stats:{id}"), move || {
            stream::once(async move { docker.stats(&id, Scope::System).await.ok() })
                .filter_map(|readings| async move { readings })
                .flatten()
                // An error (the container went away) ends the stream.
                .take_while(|reading| std::future::ready(reading.is_ok()))
                .filter_map(|reading| async move {
                    reading.ok().and_then(|r| Event::json("stats", &r).ok())
                })
        })
    }

    fn subscribe<S, F>(&self, key: String, make: F) -> AppResult<Subscription<String>>
    where
        S: futures_util::Stream<Item = Event> + Send + 'static,
        F: FnOnce() -> S + Send + 'static,
    {
        self.hub
            .subscribe(key, make)
            .map_err(|_| AppError::TooManyRequests { retry_after: 5 })
    }
}

/// Lines read together as `lines` events of at most [`LOG_EVENT_BYTES`]
/// (each line cut to fit [`LOG_LINE_BYTES`]). Ends at the first error.
pub fn log_events<S>(lines: S) -> impl futures_util::Stream<Item = Event> + Send + 'static
where
    S: futures_util::Stream<Item = Result<LogLine, crate::services::docker_service::DockerError>>
        + Send
        + 'static,
{
    lines
        .take_while(|line| std::future::ready(line.is_ok()))
        .filter_map(|line| async move { line.ok() })
        .ready_chunks(LOG_BATCH_LINES)
        .flat_map(|chunk| stream::iter(batches(chunk)))
        .then(|event| async move {
            let pause = event.data().len() as u64 * 1_000_000 / LOG_BYTES_PER_SECOND as u64;
            rustcms::realtime::time::sleep(std::time::Duration::from_micros(pause)).await;
            event
        })
}

/// `lines` as events of at most [`LOG_EVENT_BYTES`] of JSON each.
fn batches(lines: Vec<LogLine>) -> Vec<Event> {
    let mut events = Vec::new();
    let mut batch: Vec<LogLine> = Vec::new();
    let mut bytes = 0;
    let mut flush = |batch: &mut Vec<LogLine>| {
        if !batch.is_empty() {
            if let Ok(event) = Event::json("lines", &json!({ "lines": batch })) {
                events.push(event);
            }
            batch.clear();
        }
    };
    for mut line in lines {
        let size = cut_to_fit(&mut line);
        if bytes + size > LOG_EVENT_BYTES {
            flush(&mut batch);
            bytes = 0;
        }
        bytes += size + 1;
        batch.push(line);
    }
    flush(&mut batch);
    events
}

/// Cuts `line`'s text until it serializes to at most [`LOG_LINE_BYTES`]
/// (escaping can make JSON several times the text); its JSON size.
fn cut_to_fit(line: &mut LogLine) -> usize {
    loop {
        let size = serde_json::to_string(&*line).map_or(usize::MAX, |json| json.len());
        if size <= LOG_LINE_BYTES || line.text.is_empty() {
            return size;
        }
        let mut end = line.text.len() / 2;
        while !line.text.is_char_boundary(end) {
            end -= 1;
        }
        line.text.truncate(end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::docker_service::LogStream;

    fn line(text: String) -> LogLine {
        LogLine {
            stream: LogStream::Stdout,
            text,
        }
    }

    #[test]
    fn batches_stay_under_the_event_limit() {
        // JSON escaping can grow text: quotes and control characters are worst.
        let lines: Vec<_> = (0..40)
            .map(|i| line(format!("{i}:{}", "\"\u{1}".repeat(2_000))))
            .collect();
        let events = batches(lines);
        assert!(events.len() > 1);
        assert!(
            events
                .iter()
                .all(|e| e.name() == "lines" && e.data().len() <= 64 * 1024)
        );
        let total: usize = events
            .iter()
            .map(|e| {
                serde_json::from_str::<serde_json::Value>(e.data()).unwrap()["lines"]
                    .as_array()
                    .unwrap()
                    .len()
            })
            .sum();
        assert_eq!(total, 40);
    }

    #[test]
    fn long_lines_are_cut() {
        let events = batches(vec![line("é".repeat(LOG_LINE_BYTES))]);
        let value: serde_json::Value = serde_json::from_str(events[0].data()).unwrap();
        assert!(value["lines"][0]["text"].as_str().unwrap().len() <= LOG_LINE_BYTES);
    }
}
