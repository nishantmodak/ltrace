use ltrace_local::local::{self, resolve_home_with};
use serde_json::Value;
use std::path::PathBuf;

fn override_home() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ltrace-trigger-override")
}

fn present_platform() -> Option<PathBuf> {
    Some(PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ltrace-trigger-platform-present"))
}

fn absent_platform() -> Option<PathBuf> {
    None
}

fn ambient_ltrace_home() -> Option<PathBuf> {
    std::env::var_os("LTRACE_HOME").map(PathBuf::from)
}

#[test]
fn override_short_circuits_when_platform_default_is_none() {
    let tmp = override_home();
    std::fs::create_dir_all(&tmp).unwrap();
    let resolved = resolve_home_with(Some(tmp.clone()), absent_platform).unwrap();
    assert_eq!(
        resolved, tmp,
        "explicit override must win when the platform default lookup returns None"
    );
}

#[test]
fn override_takes_precedence_over_a_present_platform_default() {
    let tmp = override_home();
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::create_dir_all(present_platform().unwrap()).unwrap();
    let resolved = resolve_home_with(Some(tmp.clone()), present_platform).unwrap();
    assert_eq!(
        resolved, tmp,
        "override must beat a present platform default, not only an absent one"
    );
}

#[test]
fn no_override_without_env_surfaces_documented_error_when_platform_absent() {
    match ambient_ltrace_home() {
        None => {
            let err = resolve_home_with(None, absent_platform).unwrap_err();
            assert!(
                err.to_string()
                    .contains("cannot locate application data directory"),
                "expected the documented platform-default error, got: {err}"
            );
        }
        Some(home) => {
            let resolved = resolve_home_with(None, absent_platform).unwrap();
            assert_eq!(
                resolved, home,
                "with LTRACE_HOME set, the env override must win over an absent platform default"
            );
        }
    }
}

#[test]
fn no_override_falls_back_to_platform_default_when_env_unset() {
    let platform = present_platform().unwrap();
    std::fs::create_dir_all(&platform).unwrap();
    match ambient_ltrace_home() {
        None => {
            let resolved = resolve_home_with(None, present_platform).unwrap();
            assert_eq!(
                resolved, platform,
                "without LTRACE_HOME the resolver falls back to the platform default"
            );
        }
        Some(home) => {
            let resolved = resolve_home_with(None, present_platform).unwrap();
            assert_eq!(
                resolved, home,
                "with LTRACE_HOME set, the env override must beat the platform default"
            );
        }
    }
}

#[tokio::test]
async fn home_flag_is_resolved_and_used_end_to_end_by_doctor() {
    let tmp = tempfile::tempdir().unwrap();
    let _server = local::start(tmp.path(), 0).await.unwrap();
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_ltrace-dev"))
        .arg("--home")
        .arg(tmp.path())
        .arg("doctor")
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["data_dir"],
        tmp.path().to_str().unwrap(),
        "ltrace-dev must resolve --home and surface the supplied path as data_dir"
    );
}
