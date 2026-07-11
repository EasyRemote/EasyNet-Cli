//! Process-scoped v2 daemon key-service transport fixture for integration tests.
//!
//! Integration tests compile the library without `cfg(test)`, so daemon-local
//! LocalRuntime calls must cross the same framed v2 custody protocol as a
//! production daemon. The fixture deliberately models only the runtime-owner
//! `health`, `ensure`, `derive_pubkey`, and bound `sign` operations; managed
//! inventory lifecycle remains covered by the real key-service end-to-end
//! suites. A thread-owned UDS fixture avoids leaking child processes from test
//! binaries.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex, OnceLock};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ed25519_dalek::{Signer as _, SigningKey};
use rand::RngCore as _;
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};

static KEY_SERVICE: OnceLock<TestKeyService> = OnceLock::new();

struct TestKeyService {
    socket_path: std::path::PathBuf,
}

/// Publish the test process's explicit daemon key-service endpoint.
///
/// One process owns one fixture, which matches the runtime identity cache and
/// avoids an environment-variable race among parallel test functions.
pub fn install() {
    let fixture = KEY_SERVICE.get_or_init(TestKeyService::start);
    std::env::set_var("EASYNET_KEYRING_SOCKET_PATH", &fixture.socket_path);
}

impl TestKeyService {
    fn start() -> Self {
        let socket_path = std::env::temp_dir().join(format!(
            "easynet-v2-key-service-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after Unix epoch")
                .as_nanos()
        ));
        let listener = UnixListener::bind(&socket_path).expect("bind v2 key-service fixture");
        let keys = Arc::new(Mutex::new(BTreeMap::<String, SigningKey>::new()));
        let worker_keys = Arc::clone(&keys);
        std::thread::Builder::new()
            .name("test-v2-key-service".into())
            .spawn(move || run_server(listener, worker_keys))
            .expect("spawn v2 key-service fixture thread");
        Self { socket_path }
    }
}

fn run_server(listener: UnixListener, keys: Arc<Mutex<BTreeMap<String, SigningKey>>>) {
    for connection in listener.incoming() {
        let Ok(stream) = connection else {
            return;
        };
        let _ = handle_connection(stream, &keys);
    }
}

fn handle_connection(
    mut stream: UnixStream,
    keys: &Mutex<BTreeMap<String, SigningKey>>,
) -> std::io::Result<()> {
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length)?;
    let mut body = vec![0_u8; u32::from_be_bytes(length) as usize];
    stream.read_exact(&mut body)?;
    let response = serde_json::from_slice::<Value>(&body)
        .map(|request| dispatch(request, keys))
        .unwrap_or_else(|error| error_response("serde", error.to_string()));
    let encoded = serde_json::to_vec(&response).expect("encode v2 key-service response");
    stream.write_all(&(encoded.len() as u32).to_be_bytes())?;
    stream.write_all(&encoded)
}

fn dispatch(request: Value, keys: &Mutex<BTreeMap<String, SigningKey>>) -> Value {
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return error_response("parse", "request method is required");
    };
    match method {
        "health" => json!({"result": "health", "protocol_version": 2}),
        "ensure" => {
            let Some(owner) = request.get("primary_self").and_then(Value::as_str) else {
                return error_response("parse", "primary_self is required");
            };
            let public_key_b64 = ensure_public_key(owner, keys);
            json!({"result": "public_key", "public_key_b64": public_key_b64})
        }
        "derive_pubkey" => {
            let Some(owner) = request.get("self_ura").and_then(Value::as_str) else {
                return error_response("parse", "self_ura is required");
            };
            let keys = keys.lock().expect("test key-service lock");
            let Some(key) = keys.get(owner) else {
                return error_response("not_found", "runtime owner is not provisioned");
            };
            json!({
                "result": "public_key",
                "public_key_b64": BASE64_STANDARD.encode(key.verifying_key().to_bytes()),
            })
        }
        "sign" => sign_runtime_owner(request, keys),
        _ => error_response("parse", format!("unsupported v2 fixture method {method:?}")),
    }
}

fn ensure_public_key(owner: &str, keys: &Mutex<BTreeMap<String, SigningKey>>) -> String {
    let mut keys = keys.lock().expect("test key-service lock");
    let key = keys.entry(owner.to_string()).or_insert_with(|| {
        let mut seed = [0_u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut seed);
        SigningKey::from_bytes(&seed)
    });
    BASE64_STANDARD.encode(key.verifying_key().to_bytes())
}

fn sign_runtime_owner(request: Value, keys: &Mutex<BTreeMap<String, SigningKey>>) -> Value {
    let Some(owner) = request.get("self_ura").and_then(Value::as_str) else {
        return error_response("parse", "self_ura is required");
    };
    let Some(expected_public_key) = request.get("public_key_b64").and_then(Value::as_str) else {
        return error_response("parse", "public_key_b64 is required");
    };
    let Some(canonical_b64) = request.get("canonical_bytes_b64").and_then(Value::as_str) else {
        return error_response("parse", "canonical_bytes_b64 is required");
    };
    let Some(signer_policy_ref) = request.get("signer_policy_ref").and_then(Value::as_str) else {
        return error_response("parse", "signer_policy_ref is required");
    };
    let Ok(canonical) = BASE64_STANDARD.decode(canonical_b64) else {
        return error_response("base64", "canonical_bytes_b64 is not base64");
    };
    let keys = keys.lock().expect("test key-service lock");
    let Some(key) = keys.get(owner) else {
        return error_response("not_found", "runtime owner is not provisioned");
    };
    let actual_public_key = BASE64_STANDARD.encode(key.verifying_key().to_bytes());
    if expected_public_key != actual_public_key {
        return error_response("policy", "runtime public projection does not match owner");
    }
    if signer_policy_ref != runtime_signer_policy_ref(owner, &actual_public_key) {
        return error_response(
            "policy",
            "runtime signing policy does not match owner projection",
        );
    }
    json!({
        "result": "signature",
        "signature_b64": BASE64_STANDARD.encode(key.sign(&canonical).to_bytes()),
    })
}

fn runtime_signer_policy_ref(owner: &str, public_key_b64: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(owner.as_bytes());
    hasher.update(b"\0");
    hasher.update(owner.as_bytes());
    hasher.update(b"\0");
    hasher.update(public_key_b64.as_bytes());
    let digest = hasher.finalize();
    format!("daemon-key-inventory:sha256:{}", hex::encode(&digest[..16]))
}

fn error_response(kind: &str, message: impl Into<String>) -> Value {
    json!({"result": "error", "kind": kind, "message": message.into()})
}
