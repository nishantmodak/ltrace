//! Deterministic observations, not proof that a source-code change is safe.
use crate::model::Span;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Serialize)]
pub struct Finding {
    pub id: String,
    pub kind: &'static str,
    pub title: String,
    pub explanation: String,
    pub suggestion: &'static str,
    pub span_count: usize,
    pub span_ids: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub limitations: Vec<String>,
    pub analyzed_spans: usize,
}
fn attr(span: &Span, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        span.raw["attributes"].as_array()?.iter().find_map(|a| {
            if a["key"].as_str()? != *key {
                return None;
            }
            let v = &a["value"];
            v["stringValue"]
                .as_str()
                .or_else(|| v["intValue"].as_str())
                .map(str::to_owned)
                .or_else(|| v["intValue"].as_u64().map(|n| n.to_string()))
        })
    })
}
fn retry_counter(span: &Span) -> Option<u64> {
    ["retry.attempt", "http.request.resend_count"]
        .iter()
        .find_map(|key| attr(span, &[*key])?.parse().ok())
}
fn finding(
    kind: &'static str,
    title: String,
    explanation: String,
    suggestion: &'static str,
    spans: &[&Span],
) -> Finding {
    let mut ids: Vec<_> = spans.iter().map(|s| s.span_id.clone()).collect();
    ids.sort();
    ids.dedup();
    let id = format!("{kind}:{}", ids.first().map(String::as_str).unwrap_or(""));
    let span_count = ids.len();
    ids.truncate(200);
    Finding {
        id,
        kind,
        title,
        explanation,
        suggestion,
        span_count,
        span_ids: ids,
    }
}

pub fn detect(spans: &[Span]) -> Report {
    let mut repeated = BTreeMap::<(String, String, String, String), Vec<&Span>>::new();
    let mut retries = BTreeMap::<(String, String, String), Vec<&Span>>::new();
    let mut slow = Vec::new();
    let mut limitations = BTreeSet::new();
    let ids: BTreeSet<_> = spans.iter().map(|s| (&s.trace_id, &s.span_id)).collect();
    for span in spans {
        if span.dropped
            || span.interval().is_none()
            || (!span.parent_span_id.is_empty()
                && !ids.contains(&(&span.trace_id, &span.parent_span_id)))
        {
            limitations.insert(
                "Some telemetry is incomplete; findings describe observed spans only.".to_owned(),
            );
        }
        let query = attr(span, &["db.query.text", "db.statement"]);
        let database = query.is_some() || attr(span, &["db.system.name", "db.system"]).is_some();
        if let Some(query) = query.filter(|q| !q.trim().is_empty()) {
            if !span.parent_span_id.is_empty() {
                repeated
                    .entry((
                        span.trace_id.clone(),
                        span.service.clone(),
                        span.parent_span_id.clone(),
                        query.trim().to_owned(),
                    ))
                    .or_default()
                    .push(span);
            }
        } else if database {
            limitations.insert("Some database spans lack query text; repeated-query checks cannot cover those spans.".to_owned());
        }
        let http = attr(
            span,
            &[
                "http.request.method",
                "http.method",
                "http.response.status_code",
                "http.status_code",
                "url.full",
                "http.url",
            ],
        )
        .is_some();
        if let Some((start, end)) = span.interval()
            && ((database && end - start >= 100_000_000) || (http && end - start >= 250_000_000))
        {
            slow.push(span);
        }
        // Explicit retry metadata avoids mistaking similar span names for retries.
        if retry_counter(span).is_some() && !span.parent_span_id.is_empty() {
            retries
                .entry((
                    span.trace_id.clone(),
                    span.service.clone(),
                    span.parent_span_id.clone(),
                ))
                .or_default()
                .push(span);
        }
    }
    let mut findings = Vec::new();
    for group in repeated.values().filter(|g| g.len() >= 5) {
        findings.push(finding("possible_n_plus_one",format!("Possible N+1 · {} queries",group.len()),
            format!("{} database spans have identical recorded query text, the same service, and the same parent. This is a repeated-query pattern; it does not prove an N+1 bug. Literal SQL variants are not normalized.",group.len()),
            "Inspect the calling loop and input size. Consider a batch lookup if output ordering and behavior can be preserved.",group));
    }
    if !slow.is_empty() {
        findings.push(finding("slow_dependency",format!("Slow dependencies · {} spans",slow.len()),
            "Recorded SQL spans took at least 100 ms, or HTTP spans at least 250 ms. These are fixed starting thresholds, not a performance budget for your application. Overlapping span durations are not added together.".into(),
            "Inspect the individual spans and compare equivalent workloads before deciding whether to optimize.",&slow));
    }
    for group in retries.values() {
        let attempts: BTreeSet<_> = group.iter().filter_map(|s| retry_counter(s)).collect();
        if attempts.len() >= 2 && group.iter().any(|s| s.error) {
            findings.push(finding("retry_after_failure",format!("Retry after failure · {} attempts",attempts.len()),
                "Sibling spans include distinct explicit attempt counters and at least one failed attempt. A retry may be expected recovery; inspect the failure before changing retry behavior.".into(),
                "Check the exception or response status and whether the later attempt recovered.",group));
        }
    }
    if findings.len() > 50 {
        limitations.insert(
            "Showing the first 50 findings; narrow the captured workload for further inspection."
                .into(),
        );
    }
    findings.truncate(50);
    if findings.iter().any(|f| f.span_count > f.span_ids.len()) {
        limitations.insert("Each finding includes at most 200 span references; its count includes all observed matches.".into());
    }
    Report {
        findings,
        limitations: limitations.into_iter().collect(),
        analyzed_spans: spans.len(),
    }
}
