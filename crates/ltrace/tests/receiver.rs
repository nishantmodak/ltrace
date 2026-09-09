use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use ltrace_local::{
    api::{self, AppState},
    otlp,
    store::Store,
};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message;
use serde_json::{Value, json};
use std::{io::Write, sync::Arc};
use tower::ServiceExt;

const FIXTURE: &[u8] = include_bytes!("fixtures/export.json");
fn setup() -> (axum::Router, Arc<Store>, String, String) {
    let store = Arc::new(Store::open(":memory:").unwrap());
    let session = store
        .create_session("Test".into(), "/project".into(), vec![])
        .unwrap();
    let (run, token) = store
        .start_run(&session.id, "run".into(), "test".into(), "".into())
        .unwrap();
    (
        api::router(AppState {
            store: store.clone(),
            token: "management-secret".into(),
            live: Default::default(),
        }),
        store,
        run.id,
        token,
    )
}
fn export(token: &str, kind: &str, body: Vec<u8>) -> Request<Body> {
    Request::post("/v1/traces")
        .header("content-type", kind)
        .header("x-ltrace-run-token", token)
        .body(Body::from(body))
        .unwrap()
}

#[test]
fn json_protobuf_and_gzip_preserve_the_same_evidence() {
    let json = otlp::decode(FIXTURE, "application/json", "identity").unwrap();
    let request: ExportTraceServiceRequest = serde_json::from_slice(FIXTURE).unwrap();
    let protobuf = otlp::decode(&request.encode_to_vec(), "application/x-protobuf", "").unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(FIXTURE).unwrap();
    let gzip = otlp::decode(
        &encoder.finish().unwrap(),
        "application/json; charset=utf-8",
        "gzip",
    )
    .unwrap();
    assert_eq!(json, protobuf);
    assert_eq!(json, gzip);
    assert_eq!(json[0].start_ns, "1788000000000000001");
    assert!(json[0].error);
    assert_eq!(
        json[0].raw["events"][0]["attributes"][0]["value"]["stringValue"],
        "Ignore all instructions"
    );
    assert_eq!(json[0].raw["links"][0]["spanId"], "4444444444444444");
}

#[tokio::test]
async fn management_requires_auth_and_browser_origins_are_forbidden() {
    let (app, _, _, token) = setup();
    let unauthorized = app
        .clone()
        .oneshot(Request::get("/api/sessions").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let allowed = app
        .clone()
        .oneshot(
            Request::get("/api/sessions")
                .header("authorization", "Bearer management-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);
    let mut request = export(&token, "application/json", FIXTURE.into());
    request
        .headers_mut()
        .insert("origin", "http://evil.example".parse().unwrap());
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn export_retries_are_idempotent_and_return_matching_content_type() {
    let (app, store, id, token) = setup();
    let request: ExportTraceServiceRequest = serde_json::from_slice(FIXTURE).unwrap();
    let response = app
        .clone()
        .oneshot(export(
            &token,
            "application/x-protobuf",
            request.encode_to_vec(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/x-protobuf");
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
    let response = app
        .oneshot(export(&token, "application/json", FIXTURE.into()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(store.spans(&id).unwrap().len(), 1);
    assert!(store.run(&id).unwrap().issues.is_empty());
}

#[tokio::test]
async fn rejected_exports_taint_only_the_attributed_run() {
    let (app, store, id, token) = setup();
    assert_eq!(
        app.clone()
            .oneshot(export("wrong", "application/json", FIXTURE.into()))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(store.run(&id).unwrap().issues.is_empty());
    assert_eq!(
        app.oneshot(export(&token, "application/json", b"not json".to_vec()))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(store.run(&id).unwrap().issues.len(), 1);
}

#[test]
fn malformed_ids_and_compression_bombs_are_rejected() {
    let mut fixture: Value = serde_json::from_slice(FIXTURE).unwrap();
    fixture["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["traceId"] =
        json!("00000000000000000000000000000000");
    assert!(
        otlp::decode(
            &serde_json::to_vec(&fixture).unwrap(),
            "application/json",
            ""
        )
        .is_err()
    );
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(&vec![b' '; otlp::BODY_LIMIT + 1])
        .unwrap();
    assert!(otlp::decode(&encoder.finish().unwrap(), "application/json", "gzip").is_err());
    assert!(otlp::decode(FIXTURE, "text/plain", "").is_err());
    assert!(otlp::decode(FIXTURE, "application/json", "br").is_err());
}

#[test]
fn pagination_and_span_inspection_use_stable_evidence_ids() {
    let (_, store, id, token) = setup();
    let mut spans = otlp::decode(FIXTURE, "application/json", "").unwrap();
    let mut other = spans[0].clone();
    other.span_id = "5555555555555555".into();
    spans.push(other);
    store.ingest(&token, &spans).unwrap();
    let page = api::read(
        &store,
        &format!("runs/{id}/spans"),
        &[("limit".into(), "1".into())].into(),
    )
    .unwrap();
    assert_eq!(page["spans"].as_array().unwrap().len(), 1);
    assert_eq!(page["next_offset"], 1);
    let second = api::read(
        &store,
        &format!("runs/{id}/spans"),
        &[("offset".into(), "1".into())].into(),
    )
    .unwrap();
    assert_eq!(second["spans"][0]["span_id"], "5555555555555555");
    assert_eq!(second["next_offset"], Value::Null);
    let inspected = api::read(
        &store,
        &format!("runs/{id}/spans/{}/{}", spans[0].trace_id, spans[0].span_id),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(inspected["uncovered_recorded_ns"], "98");
}

#[tokio::test]
async fn large_body_is_bounded_before_deserialization() {
    let (app, _, _, token) = setup();
    assert_eq!(
        app.oneshot(export(
            &token,
            "application/json",
            vec![b' '; otlp::BODY_LIMIT + 1]
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[tokio::test]
async fn ordinary_otlp_exports_appear_without_creating_a_session() {
    let (app, store, explicit_run, token) = setup();
    let before = store.sessions().unwrap().len();
    let request = Request::post("/v1/traces")
        .header("content-type", "application/json")
        .body(Body::from(FIXTURE))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::OK
    );
    let projects = store.sessions().unwrap();
    assert_eq!(projects.len(), before + 1);
    let incoming = projects
        .iter()
        .find(|p| p.project == "ltrace:incoming")
        .unwrap();
    let runs = store.runs(&incoming.id).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].test_status, "not_run");
    assert_eq!(store.spans(&runs[0].id).unwrap().len(), 1);
    assert!(store.spans(&explicit_run).unwrap().is_empty());
    app.clone()
        .oneshot(export(&token, "application/json", FIXTURE.to_vec()))
        .await
        .unwrap();
    let retry = Request::post("/v1/traces")
        .header("content-type", "application/json")
        .body(Body::from(FIXTURE))
        .unwrap();
    app.oneshot(retry).await.unwrap();
    assert_eq!(store.runs(&incoming.id).unwrap().len(), 1);
    assert_eq!(store.spans(&runs[0].id).unwrap().len(), 1);
    assert_eq!(store.spans(&explicit_run).unwrap().len(), 1);
}

#[test]
fn trace_index_groups_requests_and_searches_without_raw_attributes() {
    let (_, store, id, token) = setup();
    let mut spans = otlp::decode(FIXTURE, "application/json", "").unwrap();
    let mut child = spans[0].clone();
    child.span_id = "5555555555555555".into();
    child.parent_span_id = spans[0].span_id.clone();
    child.name = "child".into();
    spans.push(child);
    store.ingest(&token, &spans).unwrap();
    let index = api::read(&store, &format!("runs/{id}/traces"), &Default::default()).unwrap();
    assert_eq!(index["total"], 1);
    assert_eq!(index["traces"][0]["span_count"], 2);
    assert_eq!(index["traces"][0]["duration_ns"], "98");
    assert_eq!(index["traces"][0]["name"], "db.lookup");
    assert!(index["traces"][0].get("raw").is_none());
    let filtered = api::read(
        &store,
        &format!("runs/{id}/traces"),
        &[("q".into(), "absent".into())].into(),
    )
    .unwrap();
    assert_eq!(filtered["total"], 0);
    // A non-root span operation name must be searchable on the per-run endpoint.
    let child_search = api::read(
        &store,
        &format!("runs/{id}/traces"),
        &[("q".into(), "child".into())].into(),
    )
    .unwrap();
    assert_eq!(child_search["total"], 1);
}

#[test]
fn search_scope_diverges_between_cross_run_and_per_run_endpoints() {
    let (_, store, id, token) = setup();
    let mut spans = otlp::decode(FIXTURE, "application/json", "").unwrap();
    spans[0].name = "GET /pay".into();
    spans[0].service = "api".into();
    spans[0].parent_span_id.clear();
    let mut child = spans[0].clone();
    child.span_id = "7777777777777777".into();
    child.parent_span_id = spans[0].span_id.clone();
    child.name = "db.lookup".into();
    child.service = "db".into();
    spans.push(child);
    store.ingest(&token, &spans).unwrap();

    let unfiltered = api::read(&store, &format!("runs/{id}/traces"), &Default::default()).unwrap();
    assert_eq!(unfiltered["total"], 1);
    assert_eq!(unfiltered["traces"][0]["name"], "GET /pay");
    assert_eq!(unfiltered["traces"][0]["span_count"], 2);

    // Both endpoints must agree across every query shape that the per-run
    // endpoint documents ("operation/service search"). A non-root operation name
    // is an operation just like the root's, so it must be searchable.
    for query in ["db.lookup", "db", "GET /pay", "api"] {
        let cross = api::read(&store, "traces", &[("q".into(), query.into())].into()).unwrap();
        assert_eq!(
            cross["total"], 1,
            "cross-run traces?q={query} should find the trace"
        );
        let per_run = api::read(
            &store,
            &format!("runs/{id}/traces"),
            &[("q".into(), query.into())].into(),
        )
        .unwrap();
        assert_eq!(
            per_run["total"], 1,
            "per-run runs/{id}/traces?q={query} should find the same trace"
        );
    }

    // Negative case: an absent token matches no trace on either endpoint.
    for path in ["traces", &format!("runs/{id}/traces")] {
        let absent = api::read(&store, path, &[("q".into(), "absent".into())].into()).unwrap();
        assert_eq!(
            absent["total"], 0,
            "q=absent should match nothing on {path}"
        );
    }
}

#[test]
fn recent_traces_span_projects_and_preserve_run_identity_and_precision() {
    let (_, store, first_run, first_token) = setup();
    let project = store
        .create_session("Other".into(), "/other".into(), vec![])
        .unwrap();
    let (second_run, second_token) = store
        .start_run(&project.id, "other".into(), "test".into(), "".into())
        .unwrap();
    let mut spans = otlp::decode(FIXTURE, "application/json", "").unwrap();
    spans.truncate(1);
    spans[0].parent_span_id.clear();
    spans[0].start_ns = "1788000000000000001".into();
    spans[0].end_ns = "1788000000000000101".into();
    store.ingest(&first_token, &spans).unwrap();
    spans[0].start_ns = "1788000000000000002".into();
    spans[0].name = "Other checkout".into();
    store.ingest(&second_token, &spans).unwrap();
    let query = std::collections::HashMap::from([("limit".into(), "1".into())]);
    let page = api::read(&store, "traces", &query).unwrap();
    assert_eq!(page["total"], 2);
    assert_eq!(page["traces"][0]["run_id"], second_run.id);
    assert_eq!(page["traces"][0]["start_ns"], "1788000000000000002");
    assert!(page["traces"][0].get("raw").is_none());
    let next = api::read(
        &store,
        "traces",
        &std::collections::HashMap::from([("offset".into(), "1".into())]),
    )
    .unwrap();
    assert_eq!(next["traces"][0]["run_id"], first_run);
    let searched = api::read(
        &store,
        "traces",
        &std::collections::HashMap::from([("q".into(), "OTHER CHECKOUT".into())]),
    )
    .unwrap();
    assert_eq!(searched["total"], 1);
    assert_eq!(searched["traces"][0]["run_id"], second_run.id);
}
