//! The SDK's own warnings and errors, scrubbed, as `core.log` events. Without
//! them a failed restore or sync reaches the journal as its last line only.

use std::collections::HashSet;
use std::fmt::{self, Write as _};
use std::sync::{Arc, Mutex};

use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

use crate::protocol::event;
use crate::runtime::Sink;
use crate::text::scrub_ids;

/// Distinct lines per run. A warning repeated on every sync is said once.
const MAX_LINES: usize = 200;

struct SdkLog {
    sink: Arc<Sink>,
    seen: Mutex<HashSet<String>>,
}

fn wanted(metadata: &Metadata<'_>) -> bool {
    *metadata.level() <= Level::WARN && metadata.target().starts_with("matrix_sdk")
}

#[derive(Default)]
struct Line {
    message: String,
    fields: String,
}

impl Line {
    fn record(&mut self, field: &Field, value: fmt::Arguments<'_>) {
        match field.name() {
            "message" => {
                let _ = self.message.write_fmt(value);
            }
            // A routing hint for the SDK's crash reporter, not information.
            "sentry" => {}
            name => {
                let _ = write!(self.fields, " {name}={value}");
            }
        }
    }
}

impl Visit for Line {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, format_args!("{value}"));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record(field, format_args!("{value:?}"));
    }
}

impl<S: Subscriber> Layer<S> for SdkLog {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if wanted(metadata) {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn enabled(&self, metadata: &Metadata<'_>, _: Context<'_, S>) -> bool {
        wanted(metadata)
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::WARN)
    }

    fn on_event(&self, record: &Event<'_>, _: Context<'_, S>) {
        let metadata = record.metadata();
        let mut line = Line::default();
        record.record(&mut line);
        let text = scrub_ids(&format!("{}{}", line.message, line.fields));
        {
            let Ok(mut seen) = self.seen.lock() else { return };
            if seen.len() >= MAX_LINES || !seen.insert(text.clone()) {
                return;
            }
        }
        let level = if *metadata.level() == Level::ERROR { "error" } else { "warn" };
        self.sink.emit(event(
            "core.log",
            json!({ "level": level, "target": metadata.target(), "message": text }),
        ));
    }
}

/// Once per process: a second core keeps the first one's sink.
pub fn install(sink: Arc<Sink>) {
    let layer = SdkLog {
        sink,
        seen: Mutex::new(HashSet::new()),
    };
    let _ = tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer));
}
