use crate::{analysis::summarize, model::Expectation, otlp, store::Store};
use anyhow::{Result, bail, ensure};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub token: String,
    pub live: Arc<std::sync::Mutex<Option<(String, String, u64)>>>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(|| async { Json(json!({"product":"ltrace-local", "version":env!("CARGO_PKG_VERSION"), "api_version":1})) }))
        .route("/api/{*path}", get(read_handler).post(write_handler))
        .route("/v1/traces", post(export))
        .layer(DefaultBodyLimit::max(otlp::BODY_LIMIT))
        .layer(middleware::from_fn_with_state(state.clone(), authorize))
        .with_state(state)
}

async fn authorize(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    // Native UI calls through IPC. No browser origin is permitted, even localhost.
    if request.headers().contains_key("origin")
        || request
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|h| h != "none")
    {
        return (StatusCode::FORBIDDEN, "browser requests are not permitted").into_response();
    }
    if request.uri().path() != "/v1/traces"
        && request
            .headers()
            .get("authorization")
            .and_then(|s| s.to_str().ok())
            != Some(&format!("Bearer {}", state.token))
    {
        return (StatusCode::UNAUTHORIZED, "local API credential required").into_response();
    }
    next.run(request).await
}

#[derive(Debug)]
struct ApiError(anyhow::Error);
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        Self(e)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let message = self.0.to_string();
        let code = if message == "not found" {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::BAD_REQUEST
        };
        (code, Json(json!({"error": message}))).into_response()
    }
}

async fn read_handler(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let data = tokio::task::spawn_blocking(move || read(&state.store, &path, &query))
        .await
        .map_err(|e| ApiError(e.into()))??;
    Ok(Json(data))
}
async fn write_handler(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let data = tokio::task::spawn_blocking(move || write(&state.store, &path, body))
        .await
        .map_err(|e| ApiError(e.into()))??;
    Ok(Json(data))
}

pub fn read(store: &Store, path: &str, query: &HashMap<String, String>) -> Result<Value> {
    let parts: Vec<_> = path.split('/').collect();
    match parts.as_slice() {
        ["sessions"] => Ok(json!({"sessions":store.sessions()?, "limit":200})),
        ["sessions", id] => Ok(
            json!({"session":store.session(id)?, "runs":store.runs(id)?, "notes":store.notes(id)?, "limit":200}),
        ),
        ["runs", id] => Ok(serde_json::to_value(summarize(
            store.run(id)?,
            &store.spans(id)?,
        ))?),
        ["runs", id, "spans"] => {
            store.run(id)?;
            let offset = query
                .get("offset")
                .map(|s| s.parse::<usize>())
                .transpose()?
                .unwrap_or(0);
            let limit = query
                .get("limit")
                .map(|s| s.parse::<usize>())
                .transpose()?
                .unwrap_or(100)
                .clamp(1, 200);
            let trace = query.get("trace_id");
            let spans: Vec<_> = store
                .spans(id)?
                .into_iter()
                .filter(|s| trace.is_none_or(|t| &s.trace_id == t))
                .collect();
            let total = spans.len();
            let page: Vec<_> = spans.into_iter().skip(offset).take(limit).collect();
            let next = offset.saturating_add(page.len());
            Ok(
                json!({"spans": page, "total":total, "next_offset": if next < total {Some(next)} else {None}}),
            )
        }
        ["runs", id, "spans", trace, span] => {
            store.run(id)?;
            let spans = store.spans(id)?;
            let s = spans
                .iter()
                .find(|s| &s.trace_id == trace && &s.span_id == span)
                .ok_or_else(|| anyhow::anyhow!("not found"))?;
            let uncovered = s.interval().map(|interval| {
                crate::analysis::uncovered_ns(
                    interval,
                    &spans
                        .iter()
                        .filter(|c| c.trace_id == s.trace_id && c.parent_span_id == s.span_id)
                        .filter_map(|c| c.interval())
                        .collect::<Vec<_>>(),
                )
                .to_string()
            });
            Ok(
                json!({"span":s,"uncovered_recorded_ns":uncovered,"coverage_note":"Parent time outside the union of recorded child spans. Not CPU or wait time."}),
            )
        }
        _ => bail!("not found"),
    }
}

#[derive(Deserialize, Serialize)]
pub struct CreateSession {
    pub title: String,
    pub project: String,
    #[serde(default)]
    pub expectations: Vec<Expectation>,
}
#[derive(Deserialize, Serialize)]
pub struct StartRun {
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub expectations: Option<Vec<Expectation>>,
}
#[derive(Deserialize, Serialize)]
pub struct FinishRun {
    pub exit_code: Option<i32>,
    pub issue: Option<String>,
}
#[derive(Deserialize, Serialize)]
pub struct AddNote {
    pub run_id: Option<String>,
    pub body: String,
}

pub fn write(store: &Store, path: &str, body: Value) -> Result<Value> {
    let parts: Vec<_> = path.split('/').collect();
    match parts.as_slice() {
        ["projects", "ensure"] => {
            let project = body["project"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("project path required"))?;
            let title = std::path::Path::new(project)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            Ok(serde_json::to_value(
                store.ensure_project(project, &title)?,
            )?)
        }
        ["sessions"] => {
            let b: CreateSession = serde_json::from_value(body)?;
            Ok(serde_json::to_value(store.create_session(
                b.title,
                b.project,
                b.expectations,
            )?)?)
        }
        ["sessions", id, "runs"] => {
            let b: StartRun = serde_json::from_value(body)?;
            let (run, token) = store.start_run_with_expectations(
                id,
                b.label,
                b.command,
                b.revision,
                b.expectations,
            )?;
            Ok(json!({"run":run,"export_token":token}))
        }
        ["sessions", id, "notes"] => {
            let b: AddNote = serde_json::from_value(body)?;
            Ok(serde_json::to_value(store.note(id, b.run_id, b.body)?)?)
        }
        ["runs", id, "finish"] => {
            let b: FinishRun = serde_json::from_value(body)?;
            if let Some(s) = &b.issue {
                ensure!(s.len() <= 2000, "issue too long");
            }
            Ok(serde_json::to_value(store.finish_run(
                id,
                b.exit_code,
                b.issue,
            )?)?)
        }
        _ => bail!("not found"),
    }
}

async fn export(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Bytes, axum::extract::rejection::BytesRejection>,
) -> Response {
    let token = headers
        .get("x-ltrace-run-token")
        .and_then(|s| s.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let content_type = headers
        .get("content-type")
        .and_then(|s| s.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let protobuf = content_type.split(';').next() == Some("application/x-protobuf");
    let encoding = headers
        .get("content-encoding")
        .and_then(|s| s.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let body = match body {
        Ok(body) => body,
        Err(rejection) => {
            let _ = state.store.mark_issue(
                &token,
                "An export exceeded the request limit or could not be read.",
            );
            return rejection.into_response();
        }
    };
    let result = tokio::task::spawn_blocking(move || -> Result<usize> {
        if !token.is_empty() {
            state.store.check_token(&token)?;
        }
        let spans = match otlp::decode(&body, &content_type, &encoding) {
            Ok(spans) => spans,
            Err(error) => {
                if !token.is_empty() {
                    state.store.mark_issue(
                        &token,
                        "An export was rejected; inspect exporter errors and rerun.",
                    )?;
                }
                return Err(error);
            }
        };
        if spans.is_empty() {
            return Ok(0);
        }
        let token = if token.is_empty() {
            let mut live = state
                .live
                .lock()
                .map_err(|_| anyhow::anyhow!("live stream lock poisoned"))?;
            if live.as_ref().is_some_and(|(_, _, created)| {
                crate::model::now_ms().saturating_sub(*created) > 3_600_000
            }) && let Some((id, _, _)) = live.take()
            {
                state.store.close_stream(&id)?;
            }
            if live.is_none() {
                let (run, token) = state.store.start_stream()?;
                *live = Some((run.id, token, run.started_ms));
            }
            live.as_ref().expect("stream initialized").1.clone()
        } else {
            token
        };
        match state.store.ingest(&token, &spans) {
            Ok(report) => {
                if report.rejected > 0 {
                    let mut live = state
                        .live
                        .lock()
                        .map_err(|_| anyhow::anyhow!("live stream lock poisoned"))?;
                    if live.as_ref().is_some_and(|(_, t, _)| t == &token)
                        && let Some((id, _, _)) = live.take()
                    {
                        state.store.close_stream(&id)?;
                    }
                }
                Ok(report.rejected)
            }
            Err(error) => {
                state.store.mark_issue(
                    &token,
                    "An export was rejected; inspect exporter errors and rerun.",
                )?;
                Err(error)
            }
        }
    })
    .await;
    match result {
        Ok(Ok(rejected)) => {
            use opentelemetry_proto::tonic::collector::trace::v1::{
                ExportTracePartialSuccess, ExportTraceServiceResponse,
            };
            use prost::Message;
            let response = ExportTraceServiceResponse {
                partial_success: (rejected > 0).then(|| ExportTracePartialSuccess {
                    rejected_spans: rejected as i64,
                    error_message: "Run storage limit reached".into(),
                }),
            };
            if protobuf {
                (
                    [("content-type", "application/x-protobuf")],
                    response.encode_to_vec(),
                )
                    .into_response()
            } else if rejected > 0 {
                Json(json!({"partialSuccess":{"rejectedSpans":rejected.to_string(),"errorMessage":"Run storage limit reached"}})).into_response()
            } else {
                Json(json!({})).into_response()
            }
        }
        Ok(Err(e)) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":e.to_string()})),
        )
            .into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "export worker failed").into_response(),
    }
}
