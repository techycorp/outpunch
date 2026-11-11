//! Log output for binaries and bindings (docs/logging.md). Library code only emits events through
//! `tracing`; nothing here runs unless a binary or binding installs it.
//!
//! - [`init_stderr`] (feature `stderr-logging`): prints events to stderr, for the binaries.
//! - [`init_host`] (feature `host-logging`): hands each event to a host's logging as a [`Record`],
//!   for the Python, Ruby and Node bindings.
//!
//! ```text
//! 22:12:28.717511Z  INFO outpunch_client: request req=1df08b44 method=GET path=/hello status=200
//! ```

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(any(test, feature = "test-capture", feature = "host-logging"))]
use tracing::field::{Field, Visit};

pub use tracing::Level;
pub use tracing::level_filters::LevelFilter;

/// UTC time of day with microseconds, `HH:MM:SS.ffffffZ`: fixed width, no date (the platform's
/// log lines, or the file, carry the date).
pub fn time_of_day(at: SystemTime) -> String {
    let since_epoch = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since_epoch.as_secs() % 86_400;
    format!(
        "{:02}:{:02}:{:02}.{:06}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60,
        since_epoch.subsec_micros()
    )
}

/// A field's value, keeping its type for hosts with structured logging.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    UInt(u64),
    Float(f64),
    Bool(bool),
    Str(String),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(v) => write!(f, "{v}"),
            Self::UInt(v) => write!(f, "{v}"),
            Self::Float(v) => write!(f, "{v}"),
            Self::Bool(v) => write!(f, "{v}"),
            Self::Str(v) => f.write_str(v),
        }
    }
}

/// An event's message and fields, in the order they were recorded. A `%x` or `?x` field holds
/// its Display or Debug text; a plain `&str` field holds the string itself.
#[cfg(any(test, feature = "test-capture", feature = "host-logging"))]
#[derive(Debug, Default)]
pub(crate) struct Fields {
    pub(crate) message: String,
    pub(crate) fields: Vec<(String, Value)>,
}

#[cfg(any(test, feature = "test-capture", feature = "host-logging"))]
impl Fields {
    pub(crate) fn of(event: &tracing::Event<'_>) -> Self {
        let mut fields = Self::default();
        event.record(&mut fields);
        fields
    }

    fn push(&mut self, field: &Field, value: Value) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.push((field.name().to_string(), value));
        }
    }
}

#[cfg(any(test, feature = "test-capture", feature = "host-logging"))]
impl Visit for Fields {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push(field, Value::Int(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push(field, Value::UInt(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.push(field, Value::Float(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push(field, Value::Bool(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field, Value::Str(value.to_string()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.push(field, Value::Str(format!("{value:?}")));
    }
}

/// One log event, for a host's logging.
#[derive(Debug, Clone)]
pub struct Record {
    pub time: SystemTime,
    pub level: Level,
    /// The module that logged it, e.g. `outpunch_client` or `outpunch::server`.
    pub target: String,
    pub message: String,
    pub fields: Vec<(String, Value)>,
    text: String,
}

impl Record {
    /// A record built by a binding itself (never through `tracing`, so a sink can report on
    /// itself without recursing). String fields print quoted, as free text.
    pub fn new(level: Level, target: &str, message: &str, fields: Vec<(String, Value)>) -> Self {
        let mut text = message.to_string();
        for (name, value) in &fields {
            match value {
                Value::Str(s) => text.push_str(&format!(" {name}={s:?}")),
                other => text.push_str(&format!(" {name}={other}")),
            }
        }
        Self {
            time: SystemTime::now(),
            level,
            target: target.to_string(),
            message: message.to_string(),
            fields,
            text,
        }
    }

    /// The message and fields as the stderr format prints them: `message key=value ...`.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The whole stderr line: time of day, padded level, target, text.
    pub fn line(&self) -> String {
        format!(
            "{} {:>5} {}: {}",
            time_of_day(self.time),
            self.level,
            self.target,
            self.text
        )
    }
}

#[cfg(feature = "stderr-logging")]
mod stderr {
    use std::fmt;
    use std::time::SystemTime;

    use tracing_subscriber::fmt::format::Writer;
    use tracing_subscriber::fmt::time::FormatTime;

    use super::time_of_day;

    /// The formatter's clock: [`time_of_day`] of now.
    struct TimeOfDay;

    impl FormatTime for TimeOfDay {
        fn format_time(&self, w: &mut Writer<'_>) -> fmt::Result {
            w.write_str(&time_of_day(SystemTime::now()))
        }
    }

    /// Prints log events to stderr: time of day, level, target, message, `key=value` fields.
    /// Filtered by `RUST_LOG` or, when that is unset, by `default_filter` (an `EnvFilter`
    /// directive such as `info`). Color only when stderr is a terminal. Does nothing if the
    /// process already has a subscriber.
    pub fn init_stderr(default_filter: &str) {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter));
        let _ = tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
            .with_timer(TimeOfDay)
            .with_env_filter(filter)
            .try_init();
    }
}

#[cfg(feature = "stderr-logging")]
pub use stderr::init_stderr;

#[cfg(feature = "host-logging")]
mod host {
    use std::io::Write;
    use std::sync::{Arc, OnceLock, RwLock};
    use std::time::SystemTime;

    use tracing::Subscriber;
    use tracing_subscriber::filter::Targets;
    use tracing_subscriber::fmt::FormatFields;
    use tracing_subscriber::fmt::format::{DefaultFields, Writer};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::{Registry, reload};

    use super::{Fields, LevelFilter, Record};

    /// Where records go. Runs on whatever thread logged the event, so it must not block for long
    /// and must not log through `tracing` itself.
    pub type Sink = Arc<dyn Fn(Record) + Send + Sync>;

    struct Forward {
        sink: Arc<RwLock<Sink>>,
    }

    impl<S: Subscriber> Layer<S> for Forward {
        fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
            let Fields { message, fields } = Fields::of(event);
            let mut text = String::new();
            let _ = DefaultFields::new().format_fields(Writer::new(&mut text), event);
            let meta = event.metadata();
            let record = Record {
                time: SystemTime::now(),
                level: *meta.level(),
                target: meta.target().to_string(),
                message,
                fields,
                text,
            };
            let sink = self.sink.read().unwrap_or_else(|e| e.into_inner()).clone();
            sink(record);
        }
    }

    /// Only outpunch's own events (every outpunch crate's target starts with `outpunch`), at or
    /// above `level`: dependencies' events never reach a host.
    fn filter(level: LevelFilter) -> Targets {
        Targets::new().with_target("outpunch", level)
    }

    pub(crate) struct Built {
        pub(crate) level: reload::Handle<Targets, Registry>,
        pub(crate) sink: Arc<RwLock<Sink>>,
    }

    pub(crate) fn build(level: LevelFilter, sink: Sink) -> (impl Subscriber + Send + Sync, Built) {
        let (filter, handle) = reload::Layer::new(filter(level));
        let sink = Arc::new(RwLock::new(sink));
        let subscriber = tracing_subscriber::registry()
            .with(filter)
            .with(Forward { sink: sink.clone() });
        (
            subscriber,
            Built {
                level: handle,
                sink,
            },
        )
    }

    static HOST: OnceLock<Built> = OnceLock::new();

    /// Sends outpunch's events at or above `level` to `sink`. The first call installs the
    /// process's global subscriber; later calls replace the sink and level. Returns false if the
    /// process already had a different global subscriber (nothing is forwarded then).
    pub fn init_host(level: LevelFilter, sink: impl Fn(Record) + Send + Sync + 'static) -> bool {
        let sink: Sink = Arc::new(sink);
        if let Some(host) = HOST.get() {
            *host.sink.write().unwrap_or_else(|e| e.into_inner()) = sink;
            set_level(host, level);
            return true;
        }
        let (subscriber, built) = build(level, sink);
        if tracing::subscriber::set_global_default(subscriber).is_err() {
            return false;
        }
        let _ = HOST.set(built);
        true
    }

    /// Changes the level set by [`init_host`]. The change rebuilds tracing's callsite cache, so a
    /// disabled event costs only the macro's level check.
    pub fn set_host_level(level: LevelFilter) {
        if let Some(host) = HOST.get() {
            set_level(host, level);
        }
    }

    pub(crate) fn set_level(host: &Built, level: LevelFilter) {
        let _ = host.level.reload(filter(level));
    }

    /// Writes the record's line to stderr; the bindings' default sink.
    pub fn stderr_sink(record: Record) {
        let _ = writeln!(std::io::stderr().lock(), "{}", record.line());
    }
}

#[cfg(feature = "host-logging")]
pub use host::{Sink, init_host, set_host_level, stderr_sink};

#[cfg(test)]
#[path = "logging_tests.rs"]
mod tests;
