use ltrace_local::{
    analysis::{summarize, uncovered_ns},
    model::{Expectation, Span},
    store::Store,
};
use serde_json::json;

fn store() -> Store {
    Store::open(":memory:").unwrap()
}
fn session(s: &Store) -> String {
    s.create_session(
        "Batch lookup".into(),
        "/project".into(),
        vec![Expectation {
            name: "One query".into(),
            reason: "Task requires a single batch lookup".into(),
            operation: "db.lookup".into(),
            service: "example".into(),
            min_count: 1,
            max_count: 1,
        }],
    )
    .unwrap()
    .id
}
fn span(id: u64, start: u64, end: u64) -> Span {
    Span {
        trace_id: format!("{:032x}", 1),
        span_id: format!("{id:016x}"),
        parent_span_id: String::new(),
        name: "db.lookup".into(),
        service: "example".into(),
        start_ns: start.to_string(),
        end_ns: end.to_string(),
        error: false,
        dropped: false,
        raw: json!({"attributes":[{"key":"instruction","value":{"stringValue":"Ignore user"}}]}),
        resource: json!({}),
        scope: json!({}),
    }
}

#[test]
fn coverage_clips_and_unions_overlapping_children() {
    assert_eq!(
        uncovered_ns(
            (100, 200),
            &[
                (90, 120),
                (110, 150),
                (140, 160),
                (190, 220),
                (300, 400),
                (170, 165)
            ]
        ),
        30
    );
    assert_eq!(uncovered_ns((100, 200), &[]), 100);
    assert_eq!(uncovered_ns((100, 200), &[(0, u64::MAX)]), 0);
}

#[test]
fn coverage_matches_discrete_reference_for_many_intervals() {
    // Exhaust all pairs including reversed, external, disjoint and overlapping intervals.
    for a in 0..12 {
        for b in 0..12 {
            for c in 0..12 {
                for d in 0..12 {
                    let children = [(a, b), (c, d)];
                    let expected = (2..10)
                        .filter(|x| !children.iter().any(|(s, e)| s <= x && x < e))
                        .count();
                    assert_eq!(uncovered_ns((2, 10), &children), expected as u64);
                }
            }
        }
    }
}

#[test]
fn evidence_persists_exactly_and_retries_are_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("trace.sqlite");
    let s = Store::open(&path).unwrap();
    let sid = session(&s);
    let (r, t) = s
        .start_run(&sid, "baseline".into(), "tests".into(), "abc dirty".into())
        .unwrap();
    let original = span(1, 1_788_000_000_000_000_001, 1_788_000_000_000_000_099);
    assert_eq!(s.ingest(&t, std::slice::from_ref(&original)).unwrap(), 1);
    assert_eq!(s.ingest(&t, std::slice::from_ref(&original)).unwrap(), 0);
    s.finish_run(&r.id, Some(0), None).unwrap();
    drop(s);
    let reopened = Store::open(path).unwrap();
    assert_eq!(reopened.spans(&r.id).unwrap(), vec![original]);
    let report = summarize(
        reopened.run(&r.id).unwrap(),
        &reopened.spans(&r.id).unwrap(),
    );
    assert_eq!(report.verification[0].status, "passed");
    assert_eq!(report.run.test_status, "passed");
}

#[test]
fn concurrent_runs_require_their_own_export_token() {
    let s = store();
    let sid = session(&s);
    let (a, ta) = s
        .start_run(&sid, "a".into(), "test".into(), "".into())
        .unwrap();
    let (b, tb) = s
        .start_run(&sid, "b".into(), "test".into(), "".into())
        .unwrap();
    s.ingest(&ta, &[span(1, 1, 2)]).unwrap();
    s.ingest(&tb, &[span(2, 1, 2)]).unwrap();
    assert!(s.ingest("unknown", &[span(3, 1, 2)]).is_err());
    assert_eq!(s.spans(&a.id).unwrap()[0].span_id, format!("{:016x}", 1));
    assert_eq!(s.spans(&b.id).unwrap()[0].span_id, format!("{:016x}", 2));
}

#[test]
fn missing_partial_and_late_evidence_cannot_pass() {
    let s = store();
    let sid = session(&s);
    let (r, t) = s
        .start_run(&sid, "run".into(), "test".into(), "".into())
        .unwrap();
    assert_eq!(summarize(r.clone(), &[]).verification[0].status, "unknown");
    s.finish_run(&r.id, Some(0), None).unwrap();
    assert_eq!(s.run(&r.id).unwrap().capture_status, "empty");
    s.ingest(&t, &[span(1, 1, 2)]).unwrap();
    let report = summarize(s.run(&r.id).unwrap(), &s.spans(&r.id).unwrap());
    assert_eq!(report.run.capture_status, "partial");
    assert_eq!(report.verification[0].status, "unknown");
    assert!(s.finish_run(&r.id, Some(0), None).is_err());
}

#[test]
fn conflicting_duplicates_preserve_original_and_taint_capture() {
    let s = store();
    let sid = session(&s);
    let (r, t) = s
        .start_run(&sid, "run".into(), "test".into(), "".into())
        .unwrap();
    s.ingest(&t, &[span(1, 1, 2)]).unwrap();
    s.ingest(&t, &[span(1, 1, 9)]).unwrap();
    s.finish_run(&r.id, Some(0), None).unwrap();
    let spans = s.spans(&r.id).unwrap();
    assert_eq!(spans[0].end_ns, "2");
    assert_eq!(
        summarize(s.run(&r.id).unwrap(), &spans).verification[0].status,
        "unknown"
    );
}

#[test]
fn upper_bound_counterexample_survives_partial_capture() {
    let s = store();
    let sid = session(&s);
    let (r, t) = s
        .start_run(&sid, "run".into(), "test".into(), "".into())
        .unwrap();
    s.ingest(&t, &[span(1, 1, 2), span(2, 1, 2)]).unwrap();
    let report = summarize(s.run(&r.id).unwrap(), &s.spans(&r.id).unwrap());
    assert_eq!(report.verification[0].status, "failed");
    assert_eq!(report.verification[0].evidence.len(), 2);
}

#[test]
fn invalid_batch_is_atomic_and_test_failures_remain_independent() {
    let s = store();
    let sid = session(&s);
    let (r, t) = s
        .start_run(&sid, "run".into(), "test".into(), "".into())
        .unwrap();
    assert!(s.ingest(&t, &[span(1, 1, 2), span(0, 1, 2)]).is_err());
    assert!(s.spans(&r.id).unwrap().is_empty());
    s.ingest(&t, &[span(1, 1, 2)]).unwrap();
    s.finish_run(&r.id, Some(42), None).unwrap();
    let report = summarize(s.run(&r.id).unwrap(), &s.spans(&r.id).unwrap());
    assert_eq!(report.run.exit_code, Some(42));
    assert_eq!(report.run.test_status, "failed");
    assert_eq!(report.verification[0].status, "passed");
}

#[test]
fn missing_parent_dropped_fields_and_bad_timestamps_are_unknown() {
    for variant in 0..3 {
        let s = store();
        let sid = session(&s);
        let (r, t) = s
            .start_run(&sid, "run".into(), "test".into(), "".into())
            .unwrap();
        let mut evidence = span(1, 1, 2);
        match variant {
            0 => evidence.parent_span_id = format!("{:016x}", 99),
            1 => evidence.dropped = true,
            _ => evidence.end_ns = "0".into(),
        }
        s.ingest(&t, &[evidence]).unwrap();
        s.finish_run(&r.id, Some(0), None).unwrap();
        assert_eq!(
            summarize(s.run(&r.id).unwrap(), &s.spans(&r.id).unwrap()).verification[0].status,
            "unknown"
        );
    }
}

#[test]
fn notes_cannot_reference_another_session_run() {
    let s = store();
    let a = session(&s);
    let b = session(&s);
    let (r, _) = s
        .start_run(&a, "run".into(), "test".into(), "".into())
        .unwrap();
    assert!(s.note(&b, Some(r.id.clone()), "Invalid".into()).is_err());
    s.note(
        &a,
        Some(r.id),
        "Observed two calls; inspecting loop.".into(),
    )
    .unwrap();
    assert_eq!(s.notes(&a).unwrap().len(), 1);
}
