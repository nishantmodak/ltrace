use ltrace_local::{
    model::Span,
    otlp,
    store::{MAX_RUN_BYTES, Store},
};
use serde_json::json;

fn store_and_run() -> (Store, String, String) {
    let s = Store::open(":memory:").unwrap();
    let sid = s
        .create_session("repro".into(), "/p".into(), vec![])
        .unwrap();
    let (r, token) = s
        .start_run(&sid.id, "run".into(), "test".into(), "".into())
        .unwrap();
    (s, r.id, token)
}

fn build_export(num_spans: usize, payload_chars: usize, id_offset: usize) -> Vec<u8> {
    let mut spans = Vec::new();
    for i in 0..num_spans {
        let n = id_offset + i + 1;
        let trace = format!("{:032x}", n);
        let span_id = format!("{:016x}", n);
        spans.push(json!({
            "traceId": trace, "spanId": span_id, "name": "op", "kind": 3,
            "startTimeUnixNano": "1788000000000000001", "endTimeUnixNano": "1788000000000000099",
            "attributes": [{"key": "payload", "value": {"stringValue": "\u{1F600}".repeat(payload_chars)}}],
            "status": {"code": 0}, "flags": 1
        }));
    }
    serde_json::to_vec(&json!({"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"svc"}}]},"scopeSpans":[{"scope":{"name":"sc","version":"1"},"spans":spans}]}]})).unwrap()
}

#[test]
fn per_run_byte_budget_is_honored_for_multibyte_utf8_payloads() {
    let (s, run_id, token) = store_and_run();
    let budget = MAX_RUN_BYTES;
    let payload_chars = 15_000; // 60_000 bytes of 4-byte UTF-8; span JSON ~60.5 KiB < 64 KiB per-span cap
    let spans_per_export = 30; // ~1.8 MiB per export <= 4 MiB BODY_LIMIT
    let mut exports = 0usize;
    loop {
        let body = build_export(spans_per_export, payload_chars, exports * spans_per_export);
        // production path: otlp::decode enforces the per-span 64 KiB BYTE cap.
        let decoded = otlp::decode(&body, "application/json", "").expect("decode must succeed");
        assert!(
            decoded.iter().all(|sp| {
                serde_json::to_vec(&sp.raw).unwrap().len()
                    + serde_json::to_vec(&sp.resource).unwrap().len()
                    + serde_json::to_vec(&sp.scope).unwrap().len()
                    <= 65_536
            }),
            "every span satisfies the per-span 64 KiB byte cap"
        );
        let res = s.ingest(&token, &decoded).unwrap();
        exports += 1;
        if res.inserted == 0 || exports >= 45 {
            break;
        }
    }
    let stored: Vec<Span> = s.spans(&run_id).unwrap();
    let stored_bytes: usize = stored
        .iter()
        .map(|sp| serde_json::to_vec(sp).unwrap().len())
        .sum();
    assert!(
        stored_bytes <= budget,
        "per-run byte budget violated: stored {stored_bytes} > {budget} (multi-byte UTF-8 slips past SUM(length(body)) which counts code points, not bytes)"
    );
}
