use super::*;

fn outcome(status: Option<u16>, latency_ms: u64) -> Outcome {
    Outcome {
        line: 1,
        method: "GET".into(),
        path: "/".into(),
        sent_ms: 0,
        status,
        latency_ms,
        error: status.is_none().then(|| "boom".into()),
    }
}

#[test]
fn counts_statuses_and_errors_and_takes_percentiles() {
    let outcomes: Vec<Outcome> = (1..=100)
        .map(|ms| outcome(if ms <= 90 { Some(200) } else { Some(504) }, ms))
        .chain([outcome(None, 1000)])
        .collect();

    let summary = summarize(&outcomes);

    assert_eq!(summary.requests, 101);
    assert_eq!(
        summary.by_status,
        BTreeMap::from([("200".into(), 90), ("504".into(), 10), ("error".into(), 1)])
    );
    assert_eq!(summary.p50_ms, 51);
    assert_eq!(summary.p99_ms, 100);
    assert_eq!(summary.max_ms, 1000);
}

#[test]
fn empty_run_has_zero_latencies() {
    let summary = summarize(&[]);
    assert_eq!(
        (summary.requests, summary.p50_ms, summary.max_ms),
        (0, 0, 0)
    );
}
