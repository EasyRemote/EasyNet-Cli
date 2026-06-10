// EasyNet CLI — Docker status contract
// ====================================
//
// File: tests/docker_status_contract.rs
// Description: Operator-surface contract for `easynet docker status --json`.
//
// Protocol Responsibility:
// - Pins Docker diagnostics to the canonical join-to-connected snapshot. Docker
//   is an execution environment and must not grow a second state model.
//
// Implementation Approach:
// - Run the built CLI binary with a temporary HOME that contains a persisted
//   connection-state snapshot. This verifies the user-facing command path, not
//   only clap parsing.
//
// Usage Contract:
// - The JSON output must preserve product `state_code`, interrupted transition,
//   and precise `failure.code`.
//
// Architectural Position:
// - Integration test for the CLI facade/operator boundary.

use serde_json::Value;
use std::fs;
use std::process::Command;

fn write_failed_snapshot() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("temp HOME");
    let state_dir = home.path().join(".easynet");
    fs::create_dir_all(&state_dir).expect("state dir");
    fs::write(
        state_dir.join("connection-state.json"),
        r#"{
  "state": "DAEMON_BOOT_FAILED",
  "state_code": "F550",
  "transition_id": "T09_OPEN_SELF_SESSION",
  "interrupted_transition": "T09_OPEN_SELF_SESSION",
  "failure": {
    "code": "CALLER_SIGNATURE_INVALID",
    "message": "CALLER_SIGNATURE_INVALID: rejected <self>.session",
    "stage": "T09_OPEN_SELF_SESSION",
    "retryable": false
  },
  "realm": "localhost",
  "node_id": "node-1",
  "device_ura": "easynet:///r/localhost/device/node-1",
  "hub_endpoint": "https://127.0.0.1:50443",
  "source": "test",
  "observed_at_unix_ms": 1780887351381
}"#,
    )
    .expect("write snapshot");
    home
}

#[test]
fn docker_status_json_reports_persisted_failure_snapshot() {
    let home = write_failed_snapshot();

    let output = Command::new(env!("CARGO_BIN_EXE_easynet"))
        .args(["docker", "status", "--json"])
        .env("HOME", home.path())
        .env_remove("USERPROFILE")
        .output()
        .expect("run easynet docker status --json");

    assert!(
        output.status.success(),
        "docker status failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let body: Value = serde_json::from_slice(&output.stdout).expect("json output");
    let connection = &body["connection"];
    assert_eq!(connection["state_code"], "F550");
    assert_eq!(
        connection["interrupted_transition"],
        "T09_OPEN_SELF_SESSION"
    );
    assert_eq!(connection["failure"]["code"], "CALLER_SIGNATURE_INVALID");
    assert_eq!(connection["failure"]["retryable"], false);
}

#[test]
fn docker_doctor_json_fails_on_failed_connection_snapshot() {
    let home = write_failed_snapshot();

    let output = Command::new(env!("CARGO_BIN_EXE_easynet"))
        .args(["docker", "doctor", "--json"])
        .env("HOME", home.path())
        .env_remove("USERPROFILE")
        .output()
        .expect("run easynet docker doctor --json");

    assert!(
        !output.status.success(),
        "docker doctor should fail on F550 snapshot\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let body: Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(body["connection"]["state_code"], "F550");
    let checks = body["checks"].as_array().expect("checks array");
    let connection_check = checks
        .iter()
        .find(|check| check["name"] == "connection state")
        .expect("connection state check");
    assert_eq!(connection_check["status"], "fail");
    assert!(connection_check["detail"]
        .as_str()
        .unwrap_or_default()
        .contains("CALLER_SIGNATURE_INVALID"));
}
