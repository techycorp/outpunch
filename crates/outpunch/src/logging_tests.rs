use std::time::Duration;

use super::*;

#[test]
fn time_of_day_is_fixed_width_utc_with_microseconds() {
    let at = UNIX_EPOCH
        + Duration::from_secs(19_000 * 86_400 + 3 * 3600 + 4 * 60 + 5)
        + Duration::from_micros(42);
    assert_eq!(time_of_day(at), "03:04:05.000042Z");
}

#[test]
fn time_of_day_wraps_at_midnight() {
    let at = UNIX_EPOCH + Duration::from_secs(86_400 - 1) + Duration::from_micros(999_999);
    assert_eq!(time_of_day(at), "23:59:59.999999Z");
    assert_eq!(
        time_of_day(at + Duration::from_micros(1)),
        "00:00:00.000000Z"
    );
}

#[test]
fn a_built_record_quotes_string_fields_only() {
    let record = Record::new(
        Level::WARN,
        "outpunch_ruby",
        "log records dropped",
        vec![
            ("count".into(), Value::UInt(3)),
            ("error".into(), Value::Str("queue full".into())),
        ],
    );
    assert_eq!(
        record.text(),
        "log records dropped count=3 error=\"queue full\""
    );
    assert!(
        record
            .line()
            .ends_with("  WARN outpunch_ruby: log records dropped count=3 error=\"queue full\""),
        "{}",
        record.line()
    );
}

#[cfg(feature = "host-logging")]
mod host {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use tracing::{Dispatch, debug, dispatcher, info, warn};

    use super::super::host::{build, set_level};
    use super::super::*;

    type Records = Arc<Mutex<Vec<Record>>>;

    fn host(level: LevelFilter) -> (Dispatch, super::super::host::Built, Records) {
        let records = Records::default();
        let into = records.clone();
        let (subscriber, built) = build(level, Arc::new(move |r| into.lock().unwrap().push(r)));
        (Dispatch::new(subscriber), built, records)
    }

    fn emit_samples() {
        let error = String::from("connection refused");
        info!(target: "outpunch::server", service = %"sim", conn = 2u64, "service registered");
        warn!(
            target: "outpunch_client",
            reason = %"error",
            duration_s = 4u64,
            error = error.as_str(),
            "connection closed"
        );
        info!(target: "outpunch_client", ok = true, "plain message");
    }

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn records_read_exactly_like_the_stderr_format() {
        let buffer = Buffer::default();
        let writer = buffer.clone();
        let fmt = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .without_time()
            .finish();
        dispatcher::with_default(&Dispatch::new(fmt), emit_samples);
        let printed = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();

        let (dispatch, _built, records) = host(LevelFilter::INFO);
        dispatcher::with_default(&dispatch, emit_samples);
        let ours: Vec<String> = records
            .lock()
            .unwrap()
            .iter()
            .map(|r| format!("{:>5} {}: {}", r.level, r.target, r.text()))
            .collect();

        let printed: Vec<&str> = printed.lines().map(str::trim_start).collect();
        let ours: Vec<&str> = ours.iter().map(|l| l.trim_start()).collect();
        assert_eq!(ours, printed);
        assert_eq!(ours.len(), 3);
    }

    #[test]
    fn fields_keep_their_type_and_free_text_arrives_unquoted() {
        let (dispatch, _built, records) = host(LevelFilter::INFO);
        dispatcher::with_default(&dispatch, emit_samples);
        let records = records.lock().unwrap();
        assert_eq!(records[0].message, "service registered");
        assert_eq!(
            records[0].fields,
            vec![
                ("service".into(), Value::Str("sim".into())),
                ("conn".into(), Value::UInt(2)),
            ]
        );
        assert_eq!(
            records[1].fields[2],
            ("error".into(), Value::Str("connection refused".into()))
        );
        assert_eq!(records[2].fields, vec![("ok".into(), Value::Bool(true))]);
    }

    #[test]
    fn only_outpunch_events_at_or_above_the_level_are_forwarded() {
        let (dispatch, _built, records) = host(LevelFilter::INFO);
        dispatcher::with_default(&dispatch, || {
            info!(target: "hyper::proto", "not ours");
            debug!(target: "outpunch::server", "below the level");
            info!(target: "outpunch_axum", "ours");
        });
        let messages: Vec<String> = records
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.message.clone())
            .collect();
        assert_eq!(messages, ["ours"]);
    }

    #[test]
    fn a_level_change_takes_effect_for_the_next_event() {
        let (dispatch, built, records) = host(LevelFilter::WARN);
        let emit = || {
            debug!(target: "outpunch::server", "detail");
            info!(target: "outpunch::server", "lifecycle");
        };
        dispatcher::with_default(&dispatch, emit);
        assert!(records.lock().unwrap().is_empty());

        set_level(&built, LevelFilter::DEBUG);
        dispatcher::with_default(&dispatch, emit);
        let messages: Vec<String> = records
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.message.clone())
            .collect();
        assert_eq!(messages, ["detail", "lifecycle"]);

        set_level(&built, LevelFilter::OFF);
        dispatcher::with_default(&dispatch, emit);
        assert_eq!(records.lock().unwrap().len(), 2);
    }
}
