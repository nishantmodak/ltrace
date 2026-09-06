use ltrace_local::{analysis::summarize, model::Expectation, store::Store};
use serde_json::json;

fn valid() -> Expectation {
    Expectation {
        name: "One query".into(),
        reason: "Task requires a single batch lookup".into(),
        operation: "db.lookup".into(),
        service: "example".into(),
        min_count: 1,
        max_count: 1,
    }
}

fn span(id: u64, start: u64, end: u64) -> ltrace_local::model::Span {
    ltrace_local::model::Span {
        trace_id: format!("{:032x}", 1),
        span_id: format!("{id:016x}"),
        parent_span_id: String::new(),
        name: "db.lookup".into(),
        service: "example".into(),
        start_ns: start.to_string(),
        end_ns: end.to_string(),
        error: false,
        dropped: false,
        raw: json!({}),
        resource: json!({}),
        scope: json!({}),
    }
}

#[test]
fn whitespace_only_operation_is_rejected_like_empty() {
    for ws in ["", "   ", "\t", "\n", "\r\n", " \t \n ", "\u{3000}"] {
        let mut e = valid();
        e.operation = ws.into();
        assert_eq!(
            e.validate().unwrap_err().to_string(),
            "expectation needs an operation and service",
            "operation={:?} must be rejected exactly like the empty string",
            ws
        );
    }
}

#[test]
fn whitespace_only_service_is_rejected_like_empty() {
    for ws in ["", "   ", "\t", "\n", "\r\n", " \t \n ", "\u{3000}"] {
        let mut e = valid();
        e.service = ws.into();
        assert_eq!(
            e.validate().unwrap_err().to_string(),
            "expectation needs an operation and service",
            "service={:?} must be rejected exactly like the empty string",
            ws
        );
    }
}

#[test]
fn stray_padded_values_are_accepted_and_matched_exactly_unchanged() {
    let mut e = valid();
    e.operation = " db.lookup ".into();
    e.validate()
        .expect("stray-padded value is not whitespace-only and must be accepted");
    let s = Store::open(":memory:").unwrap();
    let sid = s
        .create_session("Batch lookup".into(), "/project".into(), vec![e])
        .unwrap()
        .id;
    let (r, t) = s
        .start_run(&sid, "baseline".into(), "tests".into(), "abc".into())
        .unwrap();
    assert_eq!(
        s.run(&r.id).unwrap().expectations[0].operation,
        " db.lookup "
    );
    s.ingest(
        &t,
        &[span(
            1,
            1_788_000_000_000_000_001,
            1_788_000_000_000_000_099,
        )],
    )
    .unwrap();
    s.finish_run(&r.id, Some(0), None).unwrap();
    let report = summarize(s.run(&r.id).unwrap(), &s.spans(&r.id).unwrap());
    assert_eq!(report.verification[0].observed_count, 0);
    assert_eq!(report.verification[0].status, "unknown");
}
