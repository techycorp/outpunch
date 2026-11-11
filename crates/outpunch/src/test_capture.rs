//! Records log events in memory so tests can assert what was logged (docs/logging.md, Testing).
//! Test-only: compiled for this crate's tests and, with the `test-capture` feature, for the other
//! crates' tests. `#[tokio::test]` runs on one thread, so tasks a test spawns log into the same
//! recorder.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};

#[derive(Debug, Clone)]
pub struct Event {
    pub level: tracing::Level,
    /// The module that logged it, e.g. `outpunch_client` or `outpunch::server`.
    pub target: String,
    pub fields: BTreeMap<String, String>,
}

impl Event {
    pub fn message(&self) -> &str {
        self.field("message").unwrap_or("")
    }

    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }
}

#[derive(Clone, Default)]
pub struct Captured(Arc<Mutex<Vec<Event>>>);

impl Captured {
    pub fn events(&self) -> Vec<Event> {
        self.0.lock().unwrap().clone()
    }

    /// The first event with this message; panics with everything captured if none.
    pub fn find(&self, message: &str) -> Event {
        self.find_from("", message)
    }

    /// The first event with this message from a target starting with `target`.
    pub fn find_from(&self, target: &str, message: &str) -> Event {
        let events = self.events();
        events
            .iter()
            .find(|e| e.message() == message && e.target.starts_with(target))
            .cloned()
            .unwrap_or_else(|| panic!("no {message:?} event from {target:?} in {events:#?}"))
    }
}

struct Recorder(Captured);

impl<S: tracing::Subscriber> Layer<S> for Recorder {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let recorded = crate::logging::Fields::of(event);
        let mut fields: BTreeMap<String, String> = recorded
            .fields
            .into_iter()
            .map(|(name, value)| (name, value.to_string()))
            .collect();
        fields.insert("message".to_string(), recorded.message);
        (self.0).0.lock().unwrap().push(Event {
            level: *event.metadata().level(),
            target: event.metadata().target().to_string(),
            fields,
        });
    }
}

/// Starts recording; recording stops when the guard is dropped.
pub fn start() -> (Captured, tracing::subscriber::DefaultGuard) {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::registry().with(Recorder(captured.clone()));
    (captured, tracing::subscriber::set_default(subscriber))
}
