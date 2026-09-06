use crate::model::Span;
use anyhow::{Result, ensure};
use opentelemetry_proto::tonic::{
    collector::trace::v1::ExportTraceServiceRequest, common::v1::any_value::Value,
};
use prost::Message;
use std::io::Read;

pub const BODY_LIMIT: usize = 4 * 1024 * 1024;

pub fn decode(body: &[u8], content_type: &str, encoding: &str) -> Result<Vec<Span>> {
    let inflated;
    let body = match encoding {
        "" | "identity" => body,
        "gzip" => {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(body)
                .take((BODY_LIMIT + 1) as u64)
                .read_to_end(&mut out)?;
            ensure!(out.len() <= BODY_LIMIT, "decompressed export exceeds 4 MiB");
            inflated = out;
            &inflated
        }
        _ => anyhow::bail!("unsupported content encoding"),
    };
    ensure!(body.len() <= BODY_LIMIT, "export exceeds 4 MiB");
    let request: ExportTraceServiceRequest =
        match content_type.split(';').next().unwrap_or("").trim() {
            "application/json" => serde_json::from_slice(body)?,
            "application/x-protobuf" => ExportTraceServiceRequest::decode(body)?,
            _ => anyhow::bail!("use application/json or application/x-protobuf"),
        };
    let mut spans = vec![];
    for resource_spans in request.resource_spans {
        let resource = serde_json::to_value(&resource_spans.resource)?;
        let service = resource_spans
            .resource
            .as_ref()
            .and_then(|r| r.attributes.iter().find(|a| a.key == "service.name"))
            .and_then(|a| a.value.as_ref())
            .and_then(|a| a.value.as_ref())
            .and_then(|v| match v {
                Value::StringValue(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "unknown_service".into());
        let resource_dropped = resource_spans
            .resource
            .as_ref()
            .is_some_and(|r| r.dropped_attributes_count > 0);
        for scope_spans in resource_spans.scope_spans {
            let scope = serde_json::json!({"scope": scope_spans.scope, "schemaUrl": scope_spans.schema_url, "resourceSchemaUrl": resource_spans.schema_url});
            let scope_dropped = scope_spans
                .scope
                .as_ref()
                .is_some_and(|s| s.dropped_attributes_count > 0);
            for s in scope_spans.spans {
                ensure!(spans.len() < 10_000, "at most 10000 spans per export");
                let raw = serde_json::to_value(&s)?;
                ensure!(
                    serde_json::to_vec(&raw)?.len()
                        + serde_json::to_vec(&resource)?.len()
                        + serde_json::to_vec(&scope)?.len()
                        <= 65_536,
                    "span with resource/scope exceeds 64 KiB"
                );
                let span = Span {
                    trace_id: hex::encode(&s.trace_id),
                    span_id: hex::encode(&s.span_id),
                    parent_span_id: hex::encode(&s.parent_span_id),
                    name: s.name,
                    service: service.clone(),
                    start_ns: s.start_time_unix_nano.to_string(),
                    end_ns: s.end_time_unix_nano.to_string(),
                    error: s.status.as_ref().is_some_and(|s| s.code == 2),
                    dropped: resource_dropped
                        || scope_dropped
                        || s.dropped_attributes_count > 0
                        || s.dropped_events_count > 0
                        || s.dropped_links_count > 0
                        || s.events.iter().any(|e| e.dropped_attributes_count > 0)
                        || s.links.iter().any(|l| l.dropped_attributes_count > 0),
                    raw,
                    resource: resource.clone(),
                    scope: scope.clone(),
                };
                span.validate()?;
                spans.push(span);
            }
        }
    }
    Ok(spans)
}
