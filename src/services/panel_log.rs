//! The panel's own log, as its administrators read it (Settings → Panel →
//! Logs) when it runs from source: the last lines it wrote, kept in memory
//! beside the usual output. Installed, the panel is a Swarm service and its
//! logs are Docker's.
//!
//! [`init`] installs the global subscriber before RustCMS would (it then
//! leaves it): the same filter and format as `config/log.toml`, plus the
//! in-memory copy.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use tokio::sync::broadcast;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// How many lines are kept.
pub const KEPT: usize = 2_000;
/// The longest line kept (longer: cut).
const LINE_BYTES: usize = 4_096;

struct Ring {
    lines: Mutex<VecDeque<String>>,
    live: broadcast::Sender<String>,
}

fn ring() -> &'static Ring {
    static RING: OnceLock<Ring> = OnceLock::new();
    RING.get_or_init(|| Ring {
        lines: Mutex::new(VecDeque::with_capacity(KEPT)),
        live: broadcast::channel(256).0,
    })
}

/// Keeps `line` (cut to [`LINE_BYTES`]) and hands it to the followers.
pub fn push(line: &str) {
    let mut end = line.len().min(LINE_BYTES);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    let line = line[..end].trim_end().to_owned();
    let ring = ring();
    // Sent under the lock: a follower gets each line once (in its first
    // lines or live), in order.
    let mut lines = ring.lines.lock().unwrap_or_else(|e| e.into_inner());
    if lines.len() == KEPT {
        lines.pop_front();
    }
    lines.push_back(line.clone());
    let _ = ring.live.send(line);
}

/// The last `n` lines, oldest first, and the lines written from then on.
pub fn follow(n: usize) -> (Vec<String>, broadcast::Receiver<String>) {
    let ring = ring();
    // Subscribed under the lock: no line falls between the two.
    let lines = ring.lines.lock().unwrap_or_else(|e| e.into_inner());
    let live = ring.live.subscribe();
    let skip = lines.len().saturating_sub(n);
    (lines.iter().skip(skip).cloned().collect(), live)
}

/// Writes each formatted line to the ring (the fmt layer's writer).
#[derive(Clone, Copy)]
struct RingWriter;

impl std::io::Write for RingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for line in String::from_utf8_lossy(buf).lines() {
            if !line.trim().is_empty() {
                push(line);
            }
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RingWriter {
    type Writer = RingWriter;
    fn make_writer(&'a self) -> Self::Writer {
        *self
    }
}

/// Installs the global subscriber: `filter` (`RUST_LOG` overrides it), JSON
/// lines when `json`, and the in-memory copy (plain text, no colours).
/// Nothing when one is installed already.
pub fn init(filter: &str, json: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter));
    let output = if json {
        tracing_subscriber::fmt::layer().json().boxed()
    } else {
        tracing_subscriber::fmt::layer().boxed()
    };
    let kept = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(RingWriter);
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(output)
        .with(kept)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_lines() {
        let (_, mut live) = follow(0);
        push("first\n");
        assert_eq!(live.try_recv().unwrap(), "first");
        for i in 0..(KEPT + 5) {
            push(&format!("line {i}\n"));
        }
        let (last, _) = follow(3);
        assert_eq!(
            last,
            [
                format!("line {}", KEPT + 2),
                format!("line {}", KEPT + 3),
                format!("line {}", KEPT + 4)
            ]
        );
        assert_eq!(follow(KEPT * 2).0.len(), KEPT);
        // A follower too slow misses lines (it is told), never blocks.
        assert!(matches!(
            live.try_recv(),
            Err(broadcast::error::TryRecvError::Lagged(_))
        ));
        push(&"é".repeat(LINE_BYTES));
        assert!(follow(1).0[0].len() <= LINE_BYTES);
    }
}
