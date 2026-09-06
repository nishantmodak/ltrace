use ltrace_local::{api, findings, model::Span, otlp, store::Store};
use serde_json::json;

fn query(id: usize, parent: &str, query: Option<&str>) -> Span {
    let mut attributes = vec![json!({"key":"db.system","value":{"stringValue":"postgresql"}})];
    if let Some(q) = query {
        attributes.push(json!({"key":"db.statement","value":{"stringValue":q}}));
    }
    Span {
        trace_id: "11111111111111111111111111111111".into(),
        span_id: format!("{id:016x}"),
        parent_span_id: parent.into(),
        name: "SELECT".into(),
        service: "test".into(),
        start_ns: "1000000000".into(),
        end_ns: "1001000000".into(),
        error: false,
        dropped: false,
        raw: json!({"attributes":attributes}),
        resource: json!({}),
        scope: json!({}),
    }
}

#[test]
fn checkout_reports_repeated_queries_slow_dependencies_and_recovery() {
    let spans = otlp::decode(include_bytes!("fixtures/demo.json"), "application/json", "").unwrap();
    let root = spans.iter().find(|s| s.name == "Demo · Checkout").unwrap();
    let checkout: Vec<_> = spans
        .iter()
        .filter(|s| s.trace_id == root.trace_id)
        .cloned()
        .collect();
    let report = findings::detect(&checkout);
    assert_eq!(report.findings.len(), 3);
    assert_eq!(report.findings[0].kind, "possible_n_plus_one");
    assert_eq!(report.findings[0].span_count, 12);
    assert_eq!(report.findings[1].kind, "slow_dependency");
    assert_eq!(report.findings[1].span_count, 2);
    assert_eq!(report.findings[2].kind, "retry_after_failure");
    assert_eq!(report.findings[2].span_count, 2);
    for f in &report.findings {
        assert!(
            f.span_ids
                .iter()
                .all(|id| checkout.iter().any(|s| &s.span_id == id))
        );
    }
    let health: Vec<_> = spans
        .iter()
        .filter(|s| s.trace_id != root.trace_id)
        .cloned()
        .collect();
    assert!(findings::detect(&health).findings.is_empty());
}
#[test]
fn repeated_queries_require_matching_parent_service_trace_and_recorded_text() {
    let mut spans: Vec<_> = (1..=5)
        .map(|i| query(i, "aaaaaaaaaaaaaaaa", Some("SELECT x WHERE id = ?")))
        .collect();
    assert_eq!(findings::detect(&spans).findings.len(), 1);
    spans[0].parent_span_id = "bbbbbbbbbbbbbbbb".into();
    assert!(findings::detect(&spans).findings.is_empty());
    spans[0].parent_span_id = "aaaaaaaaaaaaaaaa".into();
    spans[0].service = "different".into();
    assert!(findings::detect(&spans).findings.is_empty());
    spans[0].service = "test".into();
    spans[0].trace_id = "22222222222222222222222222222222".into();
    assert!(findings::detect(&spans).findings.is_empty());
    let missing: Vec<_> = (1..=10)
        .map(|i| query(i, "aaaaaaaaaaaaaaaa", None))
        .collect();
    let report = findings::detect(&missing);
    assert!(report.findings.is_empty());
    assert!(
        report
            .limitations
            .iter()
            .any(|s| s.contains("lack query text"))
    );
}
#[test]
fn slow_thresholds_require_dependency_attributes_and_valid_intervals() {
    let mut span = query(1, "", None);
    span.end_ns = "1099999999".into();
    assert!(findings::detect(&[span.clone()]).findings.is_empty());
    span.end_ns = "1100000000".into();
    assert_eq!(
        findings::detect(&[span.clone()]).findings[0].kind,
        "slow_dependency"
    );
    span.end_ns = "999999999".into();
    let report = findings::detect(&[span]);
    assert!(report.findings.is_empty());
    assert!(!report.limitations.is_empty());
}
#[test]
fn repeated_errors_without_attempt_metadata_are_not_labeled_retries() {
    let mut spans: Vec<_> = (1..=3)
        .map(|i| query(i, "aaaaaaaaaaaaaaaa", None))
        .collect();
    for span in &mut spans {
        span.error = true;
    }
    assert!(findings::detect(&spans).findings.is_empty());
}
#[test]
fn large_findings_bound_references_without_hiding_observed_count() {
    let spans: Vec<_> = (1..=201)
        .map(|i| query(i, "aaaaaaaaaaaaaaaa", Some("SELECT x")))
        .collect();
    let report = findings::detect(&spans);
    assert_eq!(report.findings[0].span_count, 201);
    assert_eq!(report.findings[0].span_ids.len(), 200);
    assert!(
        report
            .limitations
            .iter()
            .any(|s| s.contains("200 span references"))
    );
}
#[test]
fn api_analyzes_selected_trace_and_retains_evidence_ids() {
    let store = Store::open(":memory:").unwrap();
    let session = store
        .create_session("Demo".into(), "/demo".into(), vec![])
        .unwrap();
    let (run, token) = store
        .start_run(&session.id, "demo".into(), "demo".into(), "".into())
        .unwrap();
    let spans = otlp::decode(include_bytes!("fixtures/demo.json"), "application/json", "").unwrap();
    store.ingest(&token, &spans).unwrap();
    let trace = &spans
        .iter()
        .find(|s| s.name == "Demo · Checkout")
        .unwrap()
        .trace_id;
    let report = api::read(
        &store,
        &format!("runs/{}/traces/{trace}/findings", run.id),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(report["analyzed_spans"], 30);
    assert_eq!(report["findings"][0]["span_count"], 12);
    let span_id = report["findings"][0]["span_ids"][0].as_str().unwrap();
    assert_eq!(
        store.span(&run.id, trace, span_id).unwrap().name,
        "SELECT catalog item"
    );
}
