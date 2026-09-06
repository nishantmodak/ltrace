use crate::model::{Run, Span};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Serialize)]
pub struct Verification {
    pub name: String,
    pub reason: String,
    pub status: &'static str,
    pub observed_count: usize,
    pub expected_min: u32,
    pub expected_max: u32,
    pub evidence: Vec<String>,
    pub explanation: String,
}

#[derive(Debug, Serialize)]
pub struct Operation {
    pub service: String,
    pub name: String,
    pub count: usize,
    pub errors: usize,
    pub total_duration_ns: String,
    pub sample_span_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub run: Run,
    pub span_count: usize,
    pub trace_count: usize,
    pub issues: Vec<String>,
    pub operations: Vec<Operation>,
    pub operations_total: usize,
    pub verification: Vec<Verification>,
}

/// Child intervals are clipped, sorted and unioned. This is recorded coverage,
/// never an estimate of CPU usage or proof of a wait.
pub fn uncovered_ns(parent: (u64, u64), children: &[(u64, u64)]) -> u64 {
    let (start, end) = parent;
    if end < start {
        return 0;
    }
    let mut intervals: Vec<_> = children
        .iter()
        .map(|&(s, e)| (s.max(start), e.min(end)))
        .filter(|&(s, e)| e > s)
        .collect();
    intervals.sort_unstable();
    let mut cursor = start;
    let mut covered = 0;
    for (s, e) in intervals {
        if e > cursor {
            covered += e - s.max(cursor);
            cursor = e;
        }
    }
    end - start - covered
}

pub fn summarize(run: Run, spans: &[Span]) -> Summary {
    let ids: BTreeSet<_> = spans.iter().map(|s| (&s.trace_id, &s.span_id)).collect();
    let mut issues: BTreeSet<String> = run.issues.iter().cloned().collect();
    if spans.is_empty() {
        issues.insert(
            "No spans captured; instrumentation, sampling and flush are unverified.".into(),
        );
    }
    for s in spans {
        if s.interval().is_none() {
            issues.insert("Invalid or missing span timestamps.".into());
        }
        if s.dropped {
            issues.insert("SDK reported dropped telemetry fields.".into());
        }
        if !s.parent_span_id.is_empty() && !ids.contains(&(&s.trace_id, &s.parent_span_id)) {
            issues.insert("Referenced parents are missing; capture may be incomplete or cross a service boundary.".into());
        }
    }
    let complete = run.capture_status == "settled" && issues.is_empty();
    let verification = run.expectations.iter().map(|e| {
        let matched: Vec<_> = spans.iter().filter(|s| s.service == e.service && s.name == e.operation).collect();
        let count = matched.len();
        // Exceeding an upper bound is already a counterexample, even in a partial capture.
        // Absence alone cannot establish zero executions or adequate instrumentation.
        let status = if count > e.max_count as usize { "failed" }
            else if !complete || count == 0 { "unknown" }
            else if count < e.min_count as usize { "failed" } else { "passed" };
        Verification {
            name: e.name.clone(), reason: e.reason.clone(), status,
            observed_count: count, expected_min: e.min_count, expected_max: e.max_count,
            evidence: matched.iter().take(20).map(|s| format!("{}/{}", s.trace_id, s.span_id)).collect(),
            explanation: match status {
                "passed" => "Observed count matches this run's contract; functional test result is separate.",
                "failed" => "Observed span count violates this run's contract.",
                _ => "Insufficient evidence: capture is unsettled, incomplete, or no matching span was observed.",
            }.into(),
        }
    }).collect();
    let mut groups: BTreeMap<(&str, &str), Vec<&Span>> = BTreeMap::new();
    for s in spans {
        groups.entry((&s.service, &s.name)).or_default().push(s);
    }
    let operations_total = groups.len();
    let mut operations: Vec<_> = groups
        .into_iter()
        .map(|((service, name), spans)| Operation {
            service: service.into(),
            name: name.into(),
            count: spans.len(),
            errors: spans.iter().filter(|s| s.error).count(),
            total_duration_ns: spans
                .iter()
                .filter_map(|s| s.interval())
                .map(|(s, e)| (e - s) as u128)
                .sum::<u128>()
                .to_string(),
            sample_span_ids: spans
                .iter()
                .take(5)
                .map(|s| format!("{}/{}", s.trace_id, s.span_id))
                .collect(),
        })
        .collect();
    operations.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.service.cmp(&b.service))
            .then_with(|| a.name.cmp(&b.name))
    });
    operations.truncate(100);
    Summary {
        run,
        span_count: spans.len(),
        trace_count: spans
            .iter()
            .map(|s| &s.trace_id)
            .collect::<BTreeSet<_>>()
            .len(),
        issues: issues.into_iter().collect(),
        operations,
        operations_total,
        verification,
    }
}
