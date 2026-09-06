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

fn attributes(span: &mut Span, values: &[(&str, serde_json::Value)]) {
    span.raw = json!({"attributes": values.iter().map(|(key,value)| json!({"key":key,"value":value})).collect::<Vec<_>>()});
}
fn retry(id: usize, key: &str, counter: serde_json::Value, failed: bool) -> Span {
    let mut span = query(id, "aaaaaaaaaaaaaaaa", None);
    attributes(&mut span, &[(key, counter)]);
    span.error = failed;
    span
}
fn has_retry(spans: &[Span]) -> bool {
    findings::detect(spans)
        .findings
        .iter()
        .any(|f| f.kind == "retry_after_failure")
}

#[test]
fn n_plus_one_threshold_is_five_and_both_query_attributes_are_supported() {
    let mut spans: Vec<_> = (1..=5)
        .map(|i| query(i, "aaaaaaaaaaaaaaaa", Some("SELECT name WHERE id = ?")))
        .collect();
    attributes(
        &mut spans[0],
        &[(
            "db.query.text",
            json!({"stringValue":"  SELECT name WHERE id = ?\n"}),
        )],
    );
    assert!(findings::detect(&spans[..4]).findings.is_empty());
    let report = findings::detect(&spans);
    assert_eq!(report.findings[0].span_count, 5);
    assert_eq!(
        report.findings[0].span_ids,
        spans.iter().map(|s| s.span_id.clone()).collect::<Vec<_>>()
    );
}
#[test]
fn sql_literals_case_and_internal_whitespace_are_not_normalized() {
    for variant in [
        "SELECT x WHERE id = 2",
        "select x WHERE id = 1",
        "SELECT  x WHERE id = 1",
    ] {
        let mut spans: Vec<_> = (1..=4)
            .map(|i| query(i, "aaaaaaaaaaaaaaaa", Some("SELECT x WHERE id = 1")))
            .collect();
        spans.push(query(5, "aaaaaaaaaaaaaaaa", Some(variant)));
        assert!(findings::detect(&spans).findings.is_empty(), "{variant}");
    }
}
#[test]
fn root_queries_and_blank_query_text_do_not_create_n_plus_one_findings() {
    for (parent, text) in [("", "SELECT x"), ("aaaaaaaaaaaaaaaa", " \n\t")] {
        let spans: Vec<_> = (1..=6).map(|i| query(i, parent, Some(text))).collect();
        assert!(findings::detect(&spans).findings.is_empty());
    }
}
#[test]
fn http_threshold_is_inclusive_for_every_supported_attribute() {
    for key in [
        "http.request.method",
        "http.method",
        "http.response.status_code",
        "http.status_code",
        "url.full",
        "http.url",
    ] {
        let mut span = query(1, "", None);
        let value = if key.ends_with("code") {
            json!({"intValue":"200"})
        } else if key.contains("url") {
            json!({"stringValue":"http://service.test/items"})
        } else {
            json!({"stringValue":"GET"})
        };
        attributes(&mut span, &[(key, value)]);
        span.end_ns = "1249999999".into();
        assert!(
            findings::detect(&[span.clone()]).findings.is_empty(),
            "{key} below threshold"
        );
        span.end_ns = "1250000000".into();
        let report = findings::detect(&[span]);
        assert_eq!(report.findings.len(), 1, "{key} at threshold");
        assert_eq!(report.findings[0].kind, "slow_dependency");
    }
}
#[test]
fn database_threshold_supports_current_and_legacy_attributes() {
    for key in [
        "db.query.text",
        "db.statement",
        "db.system.name",
        "db.system",
    ] {
        let mut span = query(1, "", None);
        let value = if key.contains("system") {
            "postgresql"
        } else {
            "SELECT x"
        };
        attributes(&mut span, &[(key, json!({"stringValue":value}))]);
        span.end_ns = "1100000000".into();
        assert_eq!(
            findings::detect(&[span]).findings[0].kind,
            "slow_dependency",
            "{key}"
        );
    }
}
#[test]
fn long_operation_names_alone_are_not_dependency_evidence() {
    for name in ["GET /items", "SELECT items", "payment.authorize"] {
        let mut span = query(1, "", None);
        attributes(&mut span, &[]);
        span.name = name.into();
        span.end_ns = "9000000000".into();
        assert!(findings::detect(&[span]).findings.is_empty());
    }
}
#[test]
fn retry_detection_accepts_both_counter_conventions_and_numeric_encodings() {
    for (key, first, second) in [("retry.attempt", 1, 2), ("http.request.resend_count", 0, 1)] {
        let failed = retry(1, key, json!({"intValue":first.to_string()}), true);
        let recovered = retry(2, key, json!({"intValue":second}), false);
        let report = findings::detect(&[failed, recovered]);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].kind, "retry_after_failure");
        assert_eq!(report.findings[0].span_count, 2);
    }
}
#[test]
fn retries_require_failure_distinct_counters_and_shared_context() {
    let failed = retry(1, "retry.attempt", json!({"intValue":"1"}), true);
    let recovered = retry(2, "retry.attempt", json!({"intValue":"2"}), false);
    assert!(has_retry(&[failed.clone(), recovered.clone()]));
    let mut successful = failed.clone();
    successful.error = false;
    assert!(!has_retry(&[successful, recovered.clone()]));
    let duplicate = retry(2, "retry.attempt", json!({"intValue":1}), false);
    assert!(!has_retry(&[failed.clone(), duplicate]));
    for dimension in ["parent", "service", "trace"] {
        let mut other = recovered.clone();
        match dimension {
            "parent" => other.parent_span_id = "bbbbbbbbbbbbbbbb".into(),
            "service" => other.service = "different".into(),
            _ => other.trace_id = "22222222222222222222222222222222".into(),
        }
        assert!(!has_retry(&[failed.clone(), other]), "{dimension}");
    }
}
#[test]
fn malformed_retry_counters_are_ignored_and_equal_numeric_values_are_not_retries() {
    let failed = retry(1, "retry.attempt", json!({"stringValue":"1"}), true);
    for invalid in ["not-a-number", "-1", "18446744073709551616"] {
        let other = retry(2, "retry.attempt", json!({"stringValue":invalid}), false);
        assert!(!has_retry(&[failed.clone(), other]));
    }
    let same_counter = retry(2, "retry.attempt", json!({"stringValue":"01"}), false);
    assert!(!has_retry(&[failed, same_counter]));
}
#[test]
fn incomplete_telemetry_retains_observed_findings_and_exposes_limits() {
    let mut spans: Vec<_> = (1..=5)
        .map(|i| query(i, "aaaaaaaaaaaaaaaa", Some("SELECT x")))
        .collect();
    spans[0].dropped = true;
    spans[1].end_ns = "0".into();
    let report = findings::detect(&spans);
    assert_eq!(report.findings[0].span_count, 5);
    assert!(!report.limitations.is_empty());
    assert_eq!(report.analyzed_spans, 5);
}
#[test]
fn finding_count_limit_is_explicit_and_deterministic() {
    let spans: Vec<_> = (0..51)
        .flat_map(|group| {
            (1..=5).map(move |i| {
                query(
                    group * 5 + i,
                    "aaaaaaaaaaaaaaaa",
                    Some(&format!("SELECT field_{group}")),
                )
            })
        })
        .collect();
    let report = findings::detect(&spans);
    assert_eq!(report.findings.len(), 50);
    assert!(report.limitations.iter().any(|s| s.contains("50 findings")));
    let mut reversed = spans.clone();
    reversed.reverse();
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::to_value(findings::detect(&reversed)).unwrap()
    );
}
#[test]
fn findings_api_isolates_runs_even_when_the_trace_id_is_reused() {
    let store = Store::open(":memory:").unwrap();
    let project = store
        .create_session("Test".into(), "/test".into(), vec![])
        .unwrap();
    let (first, token) = store
        .start_run(&project.id, "first".into(), "test".into(), "".into())
        .unwrap();
    let (second, other_token) = store
        .start_run(&project.id, "second".into(), "test".into(), "".into())
        .unwrap();
    let spans: Vec<_> = (1..=5)
        .map(|i| query(i, "aaaaaaaaaaaaaaaa", Some("SELECT x")))
        .collect();
    store.ingest(&token, &spans).unwrap();
    store.ingest(&other_token, &spans[..1]).unwrap();
    let read = |run: &str| {
        api::read(
            &store,
            &format!("runs/{run}/traces/{}/findings", spans[0].trace_id),
            &Default::default(),
        )
        .unwrap()
    };
    assert_eq!(read(&first.id)["findings"][0]["span_count"], 5);
    assert_eq!(read(&second.id)["analyzed_spans"], 1);
    assert!(read(&second.id)["findings"].as_array().unwrap().is_empty());
    assert!(
        api::read(
            &store,
            &format!(
                "runs/{}/traces/22222222222222222222222222222222/findings",
                first.id
            ),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn valid_resend_metadata_is_used_when_preferred_attempt_metadata_is_invalid() {
    let failed = retry(
        1,
        "http.request.resend_count",
        json!({"intValue":"0"}),
        true,
    );
    let mut recovered = retry(
        2,
        "http.request.resend_count",
        json!({"intValue":"1"}),
        false,
    );
    recovered.raw["attributes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"key":"retry.attempt","value":{"stringValue":"invalid"}}));
    assert!(has_retry(&[failed, recovered]));
}
