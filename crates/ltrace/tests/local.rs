use ltrace_local::local::{self, Client, ConnectionInfo};
use serde_json::{Value, json};
use std::time::Duration;

#[test]
fn connection_file_cannot_redirect_credentials() {
    for url in [
        "http://evil.example:4318",
        "https://127.0.0.1:4318",
        "http://localhost:4318",
        "http://user@127.0.0.1:4318",
        "http://127.0.0.1:4318/other",
    ] {
        assert!(
            Client::new(ConnectionInfo {
                url: url.into(),
                token: "secret".into()
            })
            .is_err()
        );
    }
}

#[tokio::test]
async fn local_server_lock_permissions_restart_and_persistence() {
    let tmp = tempfile::tempdir().unwrap();
    let server = local::start(tmp.path(), 0).await.unwrap();
    assert!(local::start(tmp.path(), 0).await.is_err());
    let client = Client::connect(tmp.path()).unwrap();
    client.health().await.unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(tmp.path().join("connection.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let session = client
        .write("sessions", json!({"title":"Restart","project":"/tmp"}))
        .await
        .unwrap();
    let sid = session["id"].as_str().unwrap();
    let run = client
        .write(
            &format!("sessions/{sid}/runs"),
            json!({"label":"run","command":"test"}),
        )
        .await
        .unwrap();
    let rid = run["run"]["id"].as_str().unwrap();
    drop(server);
    assert!(!tmp.path().join("connection.json").exists());
    let _server = local::start(tmp.path(), 0).await.unwrap();
    let client = Client::connect(tmp.path()).unwrap();
    let restored = client.read(&format!("runs/{rid}")).await.unwrap();
    assert_eq!(restored["run"]["test_status"], "interrupted");
    assert_eq!(restored["run"]["capture_status"], "partial");
}

#[tokio::test]
async fn capture_preserves_exit_code_empty_capture_timeout_and_spawn_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let _server = local::start(tmp.path(), 0).await.unwrap();
    let client = Client::connect(tmp.path()).unwrap();
    let session = client
        .write(
            "sessions",
            json!({"title":"CLI","project":std::env::current_dir().unwrap()}),
        )
        .await
        .unwrap();
    let sid = session["id"].as_str().unwrap();
    #[cfg(unix)]
    for (command, expected, timeout) in [
        (vec!["/bin/sh", "-c", "echo test-output; exit 7"], 7, 10),
        (vec!["/bin/sh", "-c", "sleep 20"], 2, 1),
        (vec!["/nonexistent-ltrace-command"], 2, 10),
    ] {
        let output = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::process::Command::new(env!("CARGO_BIN_EXE_ltrace-dev"))
                .arg("--home")
                .arg(tmp.path())
                .args([
                    "capture",
                    "--session",
                    sid,
                    "--settle-ms",
                    "0",
                    "--timeout-seconds",
                    &timeout.to_string(),
                    "--",
                ])
                .args(command)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            output.status.code(),
            Some(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["span_count"], 0);
        if expected == 7 {
            assert_eq!(report["run"]["exit_code"], 7);
            assert_eq!(report["run"]["capture_status"], "empty");
            assert!(String::from_utf8_lossy(&output.stderr).contains("test-output"));
        } else {
            assert_eq!(report["run"]["capture_status"], "partial");
        }
    }
}
