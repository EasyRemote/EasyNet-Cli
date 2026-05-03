// EasyNet CLI — Pages reference system: API ability surface
// ==========================================================
//
// File: src/runtime/agents/pages/api.rs
// Description: handler family for the dynamic backend API of a
//              pages project. Companion to `fetch.rs` (static
//              bytes). Where `<user>.<project>.page.fetch` is the
//              read-only deterministic projection of folder bytes,
//              `<user>.<project>.api.<verb>` is a non-deterministic
//              ability that the project author wrote into the
//              project's `api/` subfolder as a TOML manifest, and
//              that the daemon evaluates per request.
//
// Author surface (what the agent writes inside the published folder):
//
//   <project_root>/api/<verb>.toml
//
// where each `<verb>.toml` is a small declarative manifest. v0
// supports two execution modes:
//
//   1. `kind = "static_json"` — return a constant JSON value.
//      Useful for "list of products" demos where the data is part
//      of the deploy.
//   2. `kind = "echo"` — echo back the request body, optionally
//      merged with a static `extra` object. Useful for stand-in
//      "checkout" or "register" endpoints where the demo just
//      records the inputs.
//
// Both modes are intentionally simple — anything beyond them is a
// non-trivial server-side ability the agent should publish through
// the existing ability registry instead of inlining as TOML. The
// purpose of this surface is to let an LLM-authored project ship
// a working backend without writing Rust.
//
// Conformance: RFC-006-B v0.6 §10 "API surface" (post-MVP) — the
//              API ability namespace fits the same Hub adapter
//              (INV-1 still holds: Hub forwards, never mutates),
//              but does NOT inherit INV-3 because API responses
//              are non-deterministic by declaration.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::os::fd::AsFd;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{json, Value};

use super::sandbox::open_beneath;
use super::state::PUBLISHED_PROJECTS;

/// One TOML manifest under `<project>/api/<verb>.toml`.
#[derive(Debug, Deserialize)]
struct ApiManifest {
    /// Execution mode. v0: `"static_json"` or `"echo"`.
    #[serde(default = "default_kind")]
    kind: String,
    /// For `kind = "static_json"`: the JSON value to return.
    #[serde(default)]
    response: Option<toml::Value>,
    /// For `kind = "echo"`: optional fields merged on top of the
    /// echoed request body before responding.
    #[serde(default)]
    extra: Option<toml::Value>,
}

fn default_kind() -> String {
    "static_json".to_string()
}

/// Read `<project>/api/<verb>.toml` through the sandbox and parse
/// it. Returns `Err` if the manifest is missing, the path is
/// outside the published root (which the sandbox enforces), or the
/// TOML is malformed.
fn load_manifest(user: &str, project_id: &str, verb: &str) -> anyhow::Result<ApiManifest> {
    if verb.is_empty() {
        anyhow::bail!("verb is empty");
    }
    if !verb.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.') {
        anyhow::bail!("verb contains invalid character: {verb:?}");
    }

    let key = (user.to_string(), project_id.to_string());
    let handle = PUBLISHED_PROJECTS
        .get(&key)
        .ok_or_else(|| {
            anyhow::anyhow!("project not published: user={user} project_id={project_id}")
        })?
        .clone();

    let rel = format!("/api/{verb}.toml");
    let mut file = open_beneath(handle.folder_fd.as_fd(), &handle.canonical_root, &rel)?;
    use std::io::Read;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .map_err(|e| anyhow::anyhow!("read api manifest: {e}"))?;
    let parsed: ApiManifest = toml::from_str(&buf)
        .map_err(|e| anyhow::anyhow!("malformed api manifest {rel}: {e}"))?;
    Ok(parsed)
}

/// Recursively convert a `toml::Value` to `serde_json::Value`. The
/// two crates have nearly identical shapes; we hand-roll the bridge
/// rather than pull in `toml::Value::to_string` round trips.
fn toml_to_json(t: toml::Value) -> Value {
    match t {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => json!(i),
        toml::Value::Float(f) => json!(f),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => {
            let mut m = serde_json::Map::new();
            for (k, v) in t {
                m.insert(k, toml_to_json(v));
            }
            Value::Object(m)
        }
    }
}

/// Top-level API handler. Dispatches on the manifest's `kind`.
///
/// args:
/// ```json
/// { "body": <arbitrary JSON>, "method": "GET" | "POST", "query": "..." }
/// ```
///
/// Where `body` is the parsed JSON request body (empty `{}` for
/// GET), `method` is the HTTP method, and `query` is the raw
/// query string for GET requests (unparsed; the manifest may
/// inspect it).
///
/// returns:
/// ```json
/// {
///   "status": 200,
///   "body":   <JSON to send back>,
///   "content_type": "application/json"
/// }
/// ```
pub fn handle_api(user: &str, project_id: &str, verb: &str, args: Value) -> anyhow::Result<Value> {
    let manifest = load_manifest(user, project_id, verb)?;
    match manifest.kind.as_str() {
        "static_json" => {
            let resp = manifest
                .response
                .map(toml_to_json)
                .unwrap_or(Value::Null);
            Ok(json!({
                "status":       200,
                "body":         resp,
                "content_type": "application/json; charset=utf-8",
            }))
        }
        "echo" => {
            let body = args.get("body").cloned().unwrap_or(Value::Null);
            let extra = manifest.extra.map(toml_to_json).unwrap_or(Value::Null);
            // Merge: extra fields overlay echoed body for object
            // shapes; non-object body is wrapped under an `input`
            // key so the response is always a JSON object.
            let merged = match (body.clone(), extra.clone()) {
                (Value::Object(mut a), Value::Object(b)) => {
                    for (k, v) in b {
                        a.insert(k, v);
                    }
                    Value::Object(a)
                }
                (Value::Object(a), Value::Null) => Value::Object(a),
                (other, Value::Object(b)) => {
                    let mut m = serde_json::Map::new();
                    m.insert("input".to_string(), other);
                    for (k, v) in b {
                        m.insert(k, v);
                    }
                    Value::Object(m)
                }
                (other, _) => other,
            };
            Ok(json!({
                "status":       200,
                "body":         merged,
                "content_type": "application/json; charset=utf-8",
            }))
        }
        other => anyhow::bail!("unsupported api manifest kind: {other:?}"),
    }
}

/// Path on disk for the api manifest of a given verb (used in
/// tests). Always relative to the published folder root.
#[allow(dead_code)]
pub fn manifest_relpath(verb: &str) -> PathBuf {
    PathBuf::from(format!("api/{verb}.toml"))
}
