use ltrace_local::local::{self, Client, ConnectionInfo};

// A client whose bearer token does not match the receiver's must surface the
// receiver's plain-text 401 reason ("local API credential required") verbatim
// rather than the generic reqwest "error decoding response body" string that
// results from forcing a JSON decode of a non-JSON error body. Guards against
// re-introducing a decode-before-status ordering in `Client::request`.
#[tokio::test]
async fn stale_token_plain_text_401_surfaces_real_reason() {
    let tmp = tempfile::tempdir().unwrap();
    let server = local::start(tmp.path(), 0).await.unwrap();
    let wrong = Client::new(ConnectionInfo {
        url: server.info.url.clone(),
        token: "00000000-0000-0000-0000-000000000000".into(),
    })
    .unwrap();
    let err = wrong.health().await.unwrap_err().to_string();
    assert!(
        err.contains("local API credential required"),
        "expected the receiver's 401 reason, got: {err}"
    );
    assert!(
        !err.contains("error decoding response body"),
        "the real 401 reason must not be masked by a JSON decode error: {err}"
    );
    // A correctly-authenticated client against the same receiver still
    // succeeds.
    Client::connect(tmp.path()).unwrap().health().await.unwrap();
}

// Regression guard for the JSON error branch: handler errors are emitted as
// `ApiError` JSON `{"error": "..."}` bodies. The fix must surface the `error`
// field for those bodies, not the raw text. Guards the non-plain-text half of
// `Client::request`'s rewritten error handling.
#[tokio::test]
async fn json_api_error_bodies_still_surface_error_field() {
    let tmp = tempfile::tempdir().unwrap();
    let _server = local::start(tmp.path(), 0).await.unwrap();
    let client = Client::connect(tmp.path()).unwrap();
    // `/api/runs/{nonexistent}` -> ApiError "not found" -> 404 with
    // `{"error":"not found"}`.
    let err = client
        .read("runs/does-not-exist")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("not found"),
        "JSON error bodies should still surface the `error` field, got: {err}"
    );
    assert!(
        !err.contains("error decoding response body"),
        "JSON error bodies must not trigger a decode failure: {err}"
    );
}
