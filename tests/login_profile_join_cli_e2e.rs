// EasyNet CLI — login/profile/join product façade E2E
// ====================================================
//
// File: tests/login_profile_join_cli_e2e.rs
// Description: Runs the built `easynet` binary against isolated HOME
//              directories to pin the product login/profile command surface.
//
// Protocol Responsibility:
// - `login` owns account session/profile projection only.
// - `join` remains compatible with token/Hub-URA forms while accepting the
//   profile-oriented product façade.
// - Realm discovery fails closed for unconfigured bare aliases.
//
// Implementation Approach:
// - Use a minimal in-process HTTP auth fixture for `/api/v1/auth/login`.
// - Assert persisted `auth.json` and `profiles.json` through the real CLI.
// - Exercise profile/realm/hub commands as user-facing subprocesses.
//
// Usage Contract:
// - These tests do not simulate daemon admission. The existing Hub URA TLS
//   join E2E covers real device membership and PrincipalLifecycle admission.
//
// Architectural Position:
// - Product-layer E2E above the existing auth and join providers.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output};
use std::thread;
use std::time::Duration;

use serde_json::Value;

fn easynet_with_home(home: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_easynet"));
    command
        .args(args)
        .env("HOME", home)
        .env_remove("USERPROFILE");
    command
}

fn run_success(home: &Path, args: &[&str]) -> Output {
    let output = easynet_with_home(home, args)
        .output()
        .expect("spawn easynet");
    assert!(
        output.status.success(),
        "easynet {:?} failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        args,
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn run_failure(home: &Path, args: &[&str]) -> Output {
    let output = easynet_with_home(home, args)
        .output()
        .expect("spawn easynet");
    assert!(
        !output.status.success(),
        "easynet {:?} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap_or_else(|error| {
        panic!("read {}: {error}", path.display());
    }))
    .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

struct FakeAuthHub {
    endpoint: String,
    handle: Option<thread::JoinHandle<()>>,
}

impl FakeAuthHub {
    const HUB_PUBLIC_KEY_B64: &'static str = "ERERERERERERERERERERERERERERERERERERERERERE=";

    fn login_once() -> Self {
        Self::scripted(vec![ExpectedRequest::json(
            "POST /api/v1/auth/login ",
            vec![r#""email":"silan""#],
            200,
            login_response("access-token-1", "refresh-token-1", "usr_silan", "silan"),
        )])
    }

    fn one_step_join_once() -> Self {
        Self::one_step_join_with_validate_body_once(canonical_pairing_credential_response(
            "${device_public_key}",
            false,
            false,
        ))
    }

    fn one_step_join_with_validate_body_once(validate_body: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake auth hub");
        let endpoint = format!("http://{}", listener.local_addr().expect("local addr"));
        let validate_body = validate_body.replace("${endpoint}", &endpoint);
        let steps = vec![
            ExpectedRequest::json(
                "POST /api/v1/auth/login ",
                vec![r#""email":"silan""#],
                200,
                login_response("access-token-1", "refresh-token-1", "usr_silan", "silan"),
            ),
            ExpectedRequest::json(
                "POST /api/v1/devices/pairing ",
                vec!["Authorization: Bearer access-token-1"],
                200,
                r#"{"pairing_token":"token_1234","realm":"acme","node_id":"dev-one"}"#,
            ),
            ExpectedRequest::json(
                "GET /api/v1/devices/pairing/token_1234/preflight",
                Vec::new(),
                200,
                format!(
                    r#"{{
  "realm": "acme",
  "node_id": "dev-one",
  "hub_public_key_b64": "{}",
  "hub_tls_ca_pem_b64": null,
  "hub_agent_ura": "easynet:///r/acme/authority"
}}"#,
                    Self::HUB_PUBLIC_KEY_B64
                ),
            ),
            ExpectedRequest::json(
                "POST /api/v1/devices/pairing/token_1234/validate ",
                vec![r#""node_id":"dev-one""#],
                200,
                validate_body,
            ),
        ];
        let handle = thread::spawn(move || run_expected_requests(listener, steps));
        Self {
            endpoint,
            handle: Some(handle),
        }
    }

    fn one_step_join_leaks_read_model_field_once() -> Self {
        Self::one_step_join_with_validate_body_once(canonical_pairing_credential_response(
            "${device_public_key}",
            true,
            false,
        ))
    }

    fn one_step_join_nullable_federated_peers_once() -> Self {
        Self::one_step_join_with_validate_body_once(canonical_pairing_credential_response(
            "${device_public_key}",
            false,
            true,
        ))
    }

    fn one_step_join_preflight_failure_once() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake auth hub");
        let endpoint = format!("http://{}", listener.local_addr().expect("local addr"));
        let steps = vec![
            ExpectedRequest::json(
                "POST /api/v1/auth/login ",
                vec![r#""email":"silan""#],
                200,
                login_response("access-token-1", "refresh-token-1", "usr_silan", "silan"),
            ),
            ExpectedRequest::json(
                "POST /api/v1/devices/pairing ",
                vec!["Authorization: Bearer access-token-1"],
                200,
                r#"{"pairing_token":"token_1234","realm":"acme","node_id":"dev-one"}"#,
            ),
            ExpectedRequest::json(
                "GET /api/v1/devices/pairing/token_1234/preflight",
                Vec::new(),
                410,
                r#"{"error":"expired"}"#,
            ),
        ];
        let handle = thread::spawn(move || run_expected_requests(listener, steps));
        Self {
            endpoint,
            handle: Some(handle),
        }
    }

    fn scripted(steps: Vec<ExpectedRequest>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake auth hub");
        let endpoint = format!("http://{}", listener.local_addr().expect("local addr"));
        let handle = thread::spawn(move || run_expected_requests(listener, steps));
        Self {
            endpoint,
            handle: Some(handle),
        }
    }
}

struct ExpectedRequest {
    starts_with: String,
    contains: Vec<String>,
    status: u16,
    body: String,
}

impl ExpectedRequest {
    fn json(
        starts_with: impl Into<String>,
        contains: Vec<&str>,
        status: u16,
        body: impl Into<String>,
    ) -> Self {
        Self {
            starts_with: starts_with.into(),
            contains: contains.into_iter().map(ToOwned::to_owned).collect(),
            status,
            body: body.into(),
        }
    }
}

fn run_expected_requests(listener: TcpListener, steps: Vec<ExpectedRequest>) {
    for step in steps {
        let (mut stream, _) = listener.accept().expect("accept fake hub request");
        let request = read_http_request(&mut stream);
        assert!(
            request.starts_with(&step.starts_with),
            "unexpected request; expected prefix {:?}, got:\n{request}",
            step.starts_with
        );
        for expected in step.contains {
            assert!(
                request.contains(&expected),
                "request missing {expected:?}, got:\n{request}"
            );
        }
        let body = render_response_body(&step.body, &request);
        write_json_status_response(&mut stream, step.status, &body);
    }
}

fn render_response_body(template: &str, request: &str) -> String {
    if !template.contains("${device_public_key}") {
        return template.to_string();
    }
    let device_public_key = request_json_body(request)
        .and_then(|body| {
            body.get("device_public_key")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .expect("validate request body carries device_public_key");
    template.replace("${device_public_key}", &device_public_key)
}

fn request_json_body(request: &str) -> Option<Value> {
    let (_, body) = request.split_once("\r\n\r\n")?;
    serde_json::from_str(body).ok()
}

fn login_response(token: &str, refresh_token: &str, user_id: &str, username: &str) -> String {
    format!(
        r#"{{
  "token": "{token}",
  "refresh_token": "{refresh_token}",
  "user": {{
    "id": "{user_id}",
    "nickname": "{username}",
    "username": "{username}"
  }}
}}"#
    )
}

fn canonical_pairing_credential_response(
    device_public_key: &str,
    leak_state_code: bool,
    nullable_federated_peers: bool,
) -> String {
    let read_model_leak = if leak_state_code {
        r#",
  "state_code": "J200""#
    } else {
        ""
    };
    let federated_peers = if nullable_federated_peers {
        "null"
    } else {
        "[]"
    };
    format!(
        r#"{{
  "node_id": "dev-one",
  "display_name": "Workstation",
  "state": "joining",
  "trust_level": "trusted",
  "device_group": "default",
  "os": "darwin",
  "arch": "arm64",
  "auth_binding": "credential_token",
  "credential_provisioned": true,
  "public_key_registered": true,
  "device_public_key": "{device_public_key}",
  "device_public_key_fingerprint": "sha256:fake-device-key",
  "credential_token": "credential-token-1",
  "hub_endpoint": "https://hub.acme.internal:50443",
  "realm": "acme",
  "username": "silan",
  "user_id": "usr_silan",
  "deploy_signature": "deploy-signature",
  "federated_peers": {federated_peers},
  "ura": "easynet:///r/acme/device/dev-one",
  "last_seen_unix_ms": 42{read_model_leak}
}}"#
    )
}

impl Drop for FakeAuthHub {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().expect("fake auth hub thread");
        }
    }
}

fn read_http_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).expect("read request");
        assert!(read > 0, "client closed before sending request");
        bytes.extend_from_slice(&buffer[..read]);
        if request_complete(&bytes) {
            return String::from_utf8(bytes).expect("utf8 request");
        }
    }
}

fn request_complete(bytes: &[u8]) -> bool {
    let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    bytes.len() >= header_end + 4 + content_length
}

fn write_json_status_response(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        410 => "Gone",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.as_bytes().len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .expect("write response");
}

#[test]
fn login_user_at_realm_creates_auth_session_and_profile_projection() {
    let home = tempfile::tempdir().expect("temp HOME");
    let hub = FakeAuthHub::login_once();

    let output = run_success(
        home.path(),
        &[
            "login",
            "silan@acme",
            "--hub",
            hub.endpoint.as_str(),
            "--password",
            "secret",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("profile: silan@acme"), "{stdout}");
    assert!(stdout.contains("easynet join"), "{stdout}");

    let state_dir = home.path().join(".easynet");
    let auth = read_json(&state_dir.join("auth.json"));
    assert_eq!(auth["email"], "silan");
    assert_eq!(auth["hub_url"], hub.endpoint);
    assert_eq!(auth["user_id"], "usr_silan");

    let profiles = read_json(&state_dir.join("profiles.json"));
    assert_eq!(profiles["current_profile"], "silan@acme");
    let profile = &profiles["profiles"]["silan@acme"];
    assert_eq!(profile["profile_name"], "silan@acme");
    assert_eq!(profile["realm_alias"], "acme");
    assert_eq!(profile["issuer"], hub.endpoint);
    assert_eq!(profile["login_hint"], "silan");
    assert_eq!(profile["subject"], "usr_silan");
    assert_eq!(profile["account_session"], "authenticated");
    assert_eq!(profile["device_membership"], "not_enrolled");

    let show = run_success(home.path(), &["profile", "show"]);
    let show_stdout = String::from_utf8_lossy(&show.stdout);
    assert!(show_stdout.contains("profile"), "{show_stdout}");
    assert!(show_stdout.contains("silan@acme"), "{show_stdout}");
    assert!(show_stdout.contains("not_enrolled"), "{show_stdout}");

    let list = run_success(home.path(), &["profile", "list"]);
    let list_stdout = String::from_utf8_lossy(&list.stdout);
    assert!(list_stdout.contains("*"), "{list_stdout}");
    assert!(list_stdout.contains("silan@acme"), "{list_stdout}");
}

#[test]
fn one_step_join_logs_in_enrolls_device_and_marks_profile_enrolled() {
    let home = tempfile::tempdir().expect("temp HOME");
    let hub = FakeAuthHub::one_step_join_once();

    let output = run_success(
        home.path(),
        &[
            "join",
            "silan@acme",
            "--hub",
            hub.endpoint.as_str(),
            "--password",
            "secret",
            "--boot",
            "no",
            "--yes",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("profile: silan@acme"), "{stdout}");

    let state_dir = home.path().join(".easynet");
    let credentials = read_json(&state_dir.join("credentials.json"));
    assert_eq!(credentials["realm"], "acme");
    assert_eq!(credentials["node_id"], "dev-one");
    assert_eq!(credentials["username"], "silan");
    assert_eq!(credentials["user_id"], "usr_silan");
    assert_eq!(credentials["hub_api_base"], hub.endpoint);

    let profiles = read_json(&state_dir.join("profiles.json"));
    let profile = &profiles["profiles"]["silan@acme"];
    assert_eq!(profile["account_session"], "authenticated");
    assert_eq!(profile["device_membership"], "enrolled");
    assert!(
        profile.get("token").is_none(),
        "profile JSON must not store tokens"
    );
    assert!(
        profile.get("refresh_token").is_none(),
        "profile JSON must not store refresh tokens"
    );
}

#[test]
fn one_step_join_rejects_pairing_credential_read_model_leak() {
    let home = tempfile::tempdir().expect("temp HOME");
    let hub = FakeAuthHub::one_step_join_leaks_read_model_field_once();

    let output = run_failure(
        home.path(),
        &[
            "join",
            "silan@acme",
            "--hub",
            hub.endpoint.as_str(),
            "--password",
            "secret",
            "--boot",
            "no",
            "--yes",
        ],
    );
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("Hub pairing credential contract violation"),
        "{combined}"
    );
    assert!(
        combined.contains("state_code"),
        "operator error chain must identify the leaked read-model field: {combined}"
    );
    assert!(
        !home.path().join(".easynet/credentials.json").exists(),
        "contract violation must not persist device credentials"
    );
}

#[test]
fn one_step_join_rejects_nullable_pairing_federated_peers() {
    let home = tempfile::tempdir().expect("temp HOME");
    let hub = FakeAuthHub::one_step_join_nullable_federated_peers_once();

    let output = run_failure(
        home.path(),
        &[
            "join",
            "silan@acme",
            "--hub",
            hub.endpoint.as_str(),
            "--password",
            "secret",
            "--boot",
            "no",
            "--yes",
        ],
    );
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("Hub pairing credential contract violation"),
        "{combined}"
    );
    assert!(
        combined.contains("federated_peers must be a JSON array"),
        "operator error chain must identify nullable federated_peers: {combined}"
    );
    assert!(
        !home.path().join(".easynet/credentials.json").exists(),
        "contract violation must not persist device credentials"
    );
}

#[test]
fn one_step_join_failure_keeps_login_profile_and_reports_retry_command() {
    let home = tempfile::tempdir().expect("temp HOME");
    let hub = FakeAuthHub::one_step_join_preflight_failure_once();

    let output = run_failure(
        home.path(),
        &[
            "join",
            "silan@acme",
            "--hub",
            hub.endpoint.as_str(),
            "--password",
            "secret",
            "--boot",
            "no",
            "--yes",
        ],
    );
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("login succeeded and profile 'silan@acme' was saved"),
        "{combined}"
    );
    assert!(
        combined.contains("easynet join --profile silan@acme"),
        "{combined}"
    );

    let profiles = read_json(&home.path().join(".easynet/profiles.json"));
    let profile = &profiles["profiles"]["silan@acme"];
    assert_eq!(profile["account_session"], "authenticated");
    assert_eq!(profile["device_membership"], "not_enrolled");
    assert!(
        !home.path().join(".easynet/credentials.json").exists(),
        "failed enrollment must not persist device credentials"
    );
}

#[test]
fn profile_commands_support_same_realm_multi_account_and_local_remove() {
    let home = tempfile::tempdir().expect("temp HOME");
    let state_dir = home.path().join(".easynet");
    fs::create_dir_all(&state_dir).expect("state dir");
    fs::write(
        state_dir.join("profiles.json"),
        r#"{
  "current_profile": "silan@acme",
  "profiles": {
    "silan@acme": {
      "profile_name": "silan@acme",
      "realm_alias": "acme",
      "realm_id": "urn:easynet:realm:01JACME",
      "issuer": "https://hub.acme.internal",
      "login_hint": "silan",
      "subject": "usr_silan",
      "credential_ref": "local-file://auth.json",
      "account_session": "authenticated",
      "device_membership": "not_enrolled"
    },
    "admin@acme": {
      "profile_name": "admin@acme",
      "realm_alias": "acme",
      "realm_id": "urn:easynet:realm:01JACME",
      "issuer": "https://hub.acme.internal",
      "login_hint": "admin",
      "subject": "usr_admin",
      "credential_ref": "local-file://auth-admin.json",
      "account_session": "logged_out",
      "device_membership": "not_enrolled"
    }
  }
}"#,
    )
    .expect("write profiles");

    run_success(home.path(), &["profile", "use", "admin@acme"]);
    let profiles = read_json(&state_dir.join("profiles.json"));
    assert_eq!(profiles["current_profile"], "admin@acme");

    let show = run_success(home.path(), &["profile", "show"]);
    let stdout = String::from_utf8_lossy(&show.stdout);
    assert!(stdout.contains("admin@acme"), "{stdout}");
    assert!(stdout.contains("logged_out"), "{stdout}");

    let env_show = easynet_with_home(home.path(), &["profile", "show"])
        .env("EASYNET_PROFILE", "silan@acme")
        .output()
        .expect("run easynet profile show with env");
    assert!(env_show.status.success());
    let env_stdout = String::from_utf8_lossy(&env_show.stdout);
    assert!(env_stdout.contains("silan@acme"), "{env_stdout}");
    assert!(env_stdout.contains("authenticated"), "{env_stdout}");

    let remove = run_success(home.path(), &["profile", "remove", "admin@acme"]);
    let remove_stdout = String::from_utf8_lossy(&remove.stdout);
    assert!(
        remove_stdout.contains("remote account session or device membership was not revoked"),
        "{remove_stdout}"
    );
    let profiles = read_json(&state_dir.join("profiles.json"));
    assert!(profiles["profiles"].get("admin@acme").is_none());
    assert!(profiles["current_profile"].is_null());
}

#[test]
fn logout_clears_account_session_without_removing_device_membership() {
    let home = tempfile::tempdir().expect("temp HOME");
    let state_dir = home.path().join(".easynet");
    fs::create_dir_all(&state_dir).expect("state dir");
    fs::write(
        state_dir.join("auth.json"),
        r#"{
  "token": "access-token-1",
  "refresh_token": "refresh-token-1",
  "hub_url": "https://hub.acme.internal",
  "email": "silan",
  "user_id": "usr_silan",
  "username": "silan"
}"#,
    )
    .expect("write auth");
    fs::write(
        state_dir.join("profiles.json"),
        r#"{
  "current_profile": "silan@acme",
  "profiles": {
    "silan@acme": {
      "profile_name": "silan@acme",
      "realm_alias": "acme",
      "issuer": "https://hub.acme.internal",
      "login_hint": "silan",
      "subject": "usr_silan",
      "credential_ref": "local-file://auth.json",
      "account_session": "authenticated",
      "device_membership": "enrolled"
    }
  }
}"#,
    )
    .expect("write profiles");

    let output = run_success(home.path(), &["logout"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("device membership was not removed"),
        "{stdout}"
    );
    assert!(
        !state_dir.join("auth.json").exists(),
        "logout must clear the secret-bearing auth session"
    );

    let profiles = read_json(&state_dir.join("profiles.json"));
    let profile = &profiles["profiles"]["silan@acme"];
    assert_eq!(profile["account_session"], "logged_out");
    assert_eq!(profile["device_membership"], "enrolled");
}

#[test]
fn join_profile_rejects_mismatched_issuer_and_same_hub_account_session() {
    let home = tempfile::tempdir().expect("temp HOME");
    let state_dir = home.path().join(".easynet");
    fs::create_dir_all(&state_dir).expect("state dir");
    fs::write(
        state_dir.join("auth.json"),
        r#"{
  "token": "access-token-1",
  "refresh_token": "refresh-token-1",
  "hub_url": "https://hub.acme.internal",
  "email": "silan",
  "user_id": "usr_silan",
  "username": "silan"
}"#,
    )
    .expect("write auth");
    fs::write(
        state_dir.join("profiles.json"),
        r#"{
  "current_profile": "admin@acme",
  "profiles": {
    "admin@acme": {
      "profile_name": "admin@acme",
      "realm_alias": "acme",
      "issuer": "https://hub.acme.internal",
      "login_hint": "admin",
      "subject": "usr_admin",
      "credential_ref": "local-file://auth.json",
      "account_session": "authenticated",
      "device_membership": "not_enrolled"
    },
    "silan@other": {
      "profile_name": "silan@other",
      "realm_alias": "other",
      "issuer": "https://hub.other.internal",
      "login_hint": "silan",
      "subject": "usr_silan",
      "credential_ref": "local-file://auth.json",
      "account_session": "authenticated",
      "device_membership": "not_enrolled"
    }
  }
}"#,
    )
    .expect("write profiles");

    let same_hub = run_failure(
        home.path(),
        &["join", "--profile", "admin@acme", "--boot", "no", "--yes"],
    );
    let same_hub_text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&same_hub.stdout),
        String::from_utf8_lossy(&same_hub.stderr)
    );
    assert!(
        same_hub_text.contains("does not match profile 'admin@acme' subject usr_admin"),
        "{same_hub_text}"
    );

    let issuer = run_failure(
        home.path(),
        &["join", "--profile", "silan@other", "--boot", "no", "--yes"],
    );
    let issuer_text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&issuer.stdout),
        String::from_utf8_lossy(&issuer.stderr)
    );
    assert!(
        issuer_text.contains("issuer https://hub.acme.internal does not match profile 'silan@other' issuer https://hub.other.internal"),
        "{issuer_text}"
    );
}

#[test]
fn realm_resolution_fails_closed_for_unconfigured_bare_aliases() {
    let home = tempfile::tempdir().expect("temp HOME");

    let official = run_success(home.path(), &["realm", "resolve", "official"]);
    let official_stdout = String::from_utf8_lossy(&official.stdout);
    assert!(
        official_stdout.contains("built-in reserved alias"),
        "{official_stdout}"
    );
    assert!(
        official_stdout.contains("https://easynet.run"),
        "{official_stdout}"
    );

    let domain = run_success(home.path(), &["realm", "resolve", "acme.com"]);
    let domain_stdout = String::from_utf8_lossy(&domain.stdout);
    assert!(
        domain_stdout.contains("domain discovery seam"),
        "{domain_stdout}"
    );
    assert!(
        domain_stdout.contains("https://acme.com"),
        "{domain_stdout}"
    );

    let bare = run_failure(home.path(), &["realm", "resolve", "acme"]);
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&bare.stdout),
        String::from_utf8_lossy(&bare.stderr)
    );
    assert!(combined.contains("not configured"), "{combined}");
    assert!(combined.contains("--hub <url>"), "{combined}");
}

#[test]
fn hub_inspect_is_read_only_and_join_help_exposes_product_facade() {
    let home = tempfile::tempdir().expect("temp HOME");

    let hub = run_success(
        home.path(),
        &["hub", "inspect", "https://hub.acme.internal"],
    );
    let stdout = String::from_utf8_lossy(&hub.stdout);
    assert!(stdout.contains("trust_write"), "{stdout}");
    assert!(stdout.contains("no"), "{stdout}");
    assert!(
        !home.path().join(".easynet/realm-trust.toml").exists(),
        "hub inspect must not write Realm trust"
    );

    let help = run_success(home.path(), &["join", "--help"]);
    let help_stdout = String::from_utf8_lossy(&help.stdout);
    assert!(help_stdout.contains("Usage: easynet join [OPTIONS] [TARGET]"));
    assert!(help_stdout.contains("Pairing token, hub URA, or '<login-hint>@<realm>'"));
    assert!(help_stdout.contains("--profile <PROFILE>"));
    assert!(help_stdout.contains("--realm <REALM>"));
}
