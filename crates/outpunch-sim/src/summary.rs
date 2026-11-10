//! Per-request results and the run summary.

use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outcome {
    /// Line in the traffic file.
    pub line: usize,
    pub method: String,
    pub path: String,
    /// When the request was sent, ms after the tunnel was up.
    pub sent_ms: u64,
    /// HTTP status, or None when the request itself failed (`error` says why).
    pub status: Option<u16>,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub requests: usize,
    /// Status (or "error") to count.
    pub by_status: BTreeMap<String, usize>,
    pub p50_ms: u64,
    pub p99_ms: u64,
    pub max_ms: u64,
}

pub fn summarize(outcomes: &[Outcome]) -> Summary {
    let by_status = outcomes.iter().fold(BTreeMap::new(), |mut counts, o| {
        let key = o
            .status
            .map_or_else(|| "error".to_string(), |s| s.to_string());
        *counts.entry(key).or_insert(0) += 1;
        counts
    });
    let mut latencies: Vec<u64> = outcomes.iter().map(|o| o.latency_ms).collect();
    latencies.sort_unstable();
    Summary {
        requests: outcomes.len(),
        by_status,
        p50_ms: percentile(&latencies, 50),
        p99_ms: percentile(&latencies, 99),
        max_ms: latencies.last().copied().unwrap_or(0),
    }
}

/// Nearest-rank percentile of sorted values; 0 for none.
fn percentile(sorted: &[u64], p: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p * sorted.len()).div_ceil(100).max(1);
    sorted[rank - 1]
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let statuses = self
            .by_status
            .iter()
            .map(|(status, n)| format!("{status}: {n}"))
            .collect::<Vec<_>>()
            .join(", ");
        write!(
            f,
            "{} requests ({statuses}); latency p50 {} ms, p99 {} ms, max {} ms",
            self.requests, self.p50_ms, self.p99_ms, self.max_ms
        )
    }
}

#[cfg(test)]
#[path = "summary_tests.rs"]
mod tests;
