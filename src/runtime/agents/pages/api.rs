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

use std::collections::BTreeSet;
use std::fs;
use std::sync::{Arc, OnceLock};

use once_cell::sync::Lazy;
use serde::Deserialize;
use serde_json::{json, Value};

use super::sandbox::open_beneath;
use super::state::PUBLISHED_PROJECTS;
use crate::runtime::ability_dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};
use crate::ura::AbilitySelector;

/// Process-wide handle to the live ability registry. Set once at
/// boot by `pages::register`; read by the `kind="ability"` branch
/// to dispatch requests directly through the in-process registry
/// instead of round-tripping through the daemon's own IPC socket
/// (which would self-deadlock).
static DISPATCH_HANDLE: Lazy<std::sync::OnceLock<Arc<OnceLock<Arc<AxonAbilityCatalog>>>>> =
    Lazy::new(std::sync::OnceLock::new);

pub(crate) fn set_dispatch_handle(handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>>) {
    let _ = DISPATCH_HANDLE.set(handle);
}

/// One TOML manifest under `<project>/api/<verb>.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApiManifest {
    /// Execution mode. v0: `"static_json"` | `"echo"` | `"ability"`.
    #[serde(default = "default_kind")]
    kind: String,
    /// For `kind = "static_json"`: the JSON value to return.
    #[serde(default)]
    response: Option<toml::Value>,
    /// For `kind = "echo"`: optional fields merged on top of the
    /// echoed request body before responding.
    #[serde(default)]
    extra: Option<toml::Value>,
    /// For `kind = "ability"`: canonical Ability URA to invoke.
    /// The HTTP request body is forwarded verbatim as that ability's
    /// args; local registry projection happens inside the daemon.
    #[serde(default)]
    ability_ura: Option<String>,
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
    if !verb
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
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
    let mut file = open_beneath(&handle.folder_handle, &handle.canonical_root, &rel)?;
    use std::io::Read;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .map_err(|e| anyhow::anyhow!("read api manifest: {e}"))?;
    let parsed: ApiManifest =
        toml::from_str(&buf).map_err(|e| anyhow::anyhow!("malformed api manifest {rel}: {e}"))?;
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
            let resp = manifest.response.map(toml_to_json).unwrap_or(Value::Null);
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
        "ability" => {
            // The manifest forwards the request to a real EasyNet
            // ability — silan's "agent writes a real backend"
            // case. Body becomes the ability's args verbatim;
            // ability response becomes the HTTP body.
            let ability_ura = manifest.ability_ura.ok_or_else(|| {
                anyhow::anyhow!(
                    "api manifest kind=ability requires `ability_ura = \"<Ability URA>\"`"
                )
            })?;
            let selector = AbilitySelector::parse(&ability_ura)
                .map_err(|e| anyhow::anyhow!("api manifest invalid ability_ura: {e}"))?;
            let body = args.get("body").cloned().unwrap_or(Value::Null);
            let invoke_args = match body {
                Value::Null => json!({}),
                v => v,
            };
            // Use the daemon's shared Axon LocalRuntime. We are
            // already inside the daemon process, so an IPC round trip
            // would self-deadlock the original request.
            let handle = DISPATCH_HANDLE.get().ok_or_else(|| {
                anyhow::anyhow!("dispatch handle not set; pages::register must run at boot")
            })?;
            let registry = handle.get().ok_or_else(|| {
                anyhow::anyhow!("dispatch handle empty; build site forgot to populate OnceLock")
            })?;
            let target = InvocationTarget {
                scope: TargetScope::Local,
                ability: selector.local_registry_ability().to_string(),
                normalized_args: invoke_args,
                call_mode: CallMode::Rpc,
                subject: Some(selector.owner_ura().to_string()),
                causal_context: None,
            };
            let result = registry.invoke_rpc_target_json(target).map_err(|e| {
                anyhow::anyhow!(
                    "ability `{}` ({}) failed: {e}",
                    selector.ability_ura(),
                    selector.local_registry_ability()
                )
            })?;
            Ok(json!({
                "status":       200,
                "body":         result,
                "content_type": "application/json; charset=utf-8",
            }))
        }
        other => anyhow::bail!("unsupported api manifest kind: {other:?}"),
    }
}

fn api_ability_name(user: &str, project_id: &str, verb: &str) -> String {
    format!("{user}.{project_id}.api.{verb}")
}

fn is_api_ability_verb(verb: &str) -> bool {
    !verb.is_empty()
        && verb
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub(crate) fn api_ability_names_for_project(user: &str, project_id: &str) -> Vec<String> {
    let key = (user.to_string(), project_id.to_string());
    let Some(handle) = PUBLISHED_PROJECTS.get(&key) else {
        return Vec::new();
    };
    let api_dir = handle.canonical_root.join("api");
    drop(handle);

    let read_dir = match fs::read_dir(&api_dir) {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            let user_field = user;
            let project_field = project_id;
            let path = api_dir.display().to_string();
            let err_msg = format!("{err}");
            crate::op_event!(
                component = pages,
                kind = api_manifest_scan_failed,
                level = "warn",
                user = user_field,
                project_id = project_field,
                path = path,
                error = err_msg,
            );
            return Vec::new();
        }
    };

    let mut names = BTreeSet::new();
    for entry in read_dir.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }
        let Some(verb) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !is_api_ability_verb(verb) {
            let user_field = user;
            let project_field = project_id;
            let verb_field = verb.to_string();
            crate::op_event!(
                component = pages,
                kind = api_manifest_skipped,
                level = "warn",
                user = user_field,
                project_id = project_field,
                verb = verb_field,
                reason = "invalid_ability_verb",
            );
            continue;
        }
        names.insert(api_ability_name(user, project_id, verb));
    }
    names.into_iter().collect()
}

pub(crate) fn register_api_abilities_for_project(
    registry: &AxonAbilityCatalog,
    user: &str,
    project_id: &str,
) -> anyhow::Result<usize> {
    let names = api_ability_names_for_project(user, project_id);
    let owner = OwnerKind::User(user.to_string());
    for name in &names {
        let Some(verb) = name.rsplit_once(".api.").map(|(_prefix, verb)| verb) else {
            continue;
        };
        let user = user.to_string();
        let project_id = project_id.to_string();
        let verb = verb.to_string();
        registry.hot_register_rpc(
            name.clone(),
            owner.clone(),
            Arc::new(move |args| handle_api(&user, &project_id, &verb, args)),
        )?;
    }
    Ok(names.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ability_dispatch::{AxonAbilityCatalog, OwnerKind};
    use crate::runtime::agents::pages::sandbox::open_directory;
    use crate::runtime::agents::pages::state::{
        ProjectHandle, Visibility, DEFAULT_FILE_SIZE_CAP, PUBLISHED_PROJECTS,
    };
    use serde_json::json;
    use std::sync::{Arc, OnceLock};
    use std::time::SystemTime;
    use tempfile::TempDir;

    fn publish_project_with_manifest(
        user: &str,
        project_id: &str,
        verb: &str,
        manifest_toml: &str,
    ) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("api")).expect("api dir");
        std::fs::write(dir.path().join(format!("api/{verb}.toml")), manifest_toml)
            .expect("manifest");
        let canonical_root = std::fs::canonicalize(dir.path()).expect("canonical root");
        let folder_handle = open_directory(&canonical_root).expect("open directory");
        PUBLISHED_PROJECTS.insert(
            (user.to_string(), project_id.to_string()),
            Arc::new(ProjectHandle {
                user: user.to_string(),
                project_id: project_id.to_string(),
                folder_handle,
                canonical_root,
                visibility: Visibility::Public,
                file_size_cap: DEFAULT_FILE_SIZE_CAP,
                started_at: SystemTime::now(),
            }),
        );
        dir
    }

    fn install_dispatch_registry_once() {
        static INIT: OnceLock<()> = OnceLock::new();
        INIT.get_or_init(|| {
            let mut reg = AxonAbilityCatalog::new();
            reg.register_rpc_with_owner(
                "demo.backend",
                OwnerKind::Agent("demo".into()),
                Arc::new(|args| Ok(json!({"ok": true, "echo": args}))),
            );
            let reg = Arc::new(reg);
            let handle = Arc::new(OnceLock::new());
            handle
                .set(reg)
                .expect("dispatch handle OnceLock should set once");
            set_dispatch_handle(handle);
        });
    }

    #[test]
    fn ability_manifest_invokes_registry_handler() {
        install_dispatch_registry_once();
        let user = "alice";
        let project_id = "todo";
        let key = (user.to_string(), project_id.to_string());
        let _dir = publish_project_with_manifest(
            user,
            project_id,
            "submit",
            "kind = \"ability\"\nability_ura = \"easynet:///r/acme/ability/alice.demo.backend\"\n",
        );

        let resp = handle_api(
            user,
            project_id,
            "submit",
            json!({
                "body": {"task": "ship windows support"},
                "method": "POST"
            }),
        )
        .expect("handle_api ok");

        assert_eq!(resp["status"], 200);
        assert_eq!(resp["body"]["ok"], true);
        assert_eq!(resp["body"]["echo"]["task"], "ship windows support");

        PUBLISHED_PROJECTS.remove(&key);
    }

    #[test]
    fn ability_manifest_rejects_retired_bare_ability_field() {
        let user = "alice-old";
        let project_id = "todo-old";
        let key = (user.to_string(), project_id.to_string());
        let _dir = publish_project_with_manifest(
            user,
            project_id,
            "submit",
            "kind = \"ability\"\nability = \"demo.backend\"\n",
        );

        let err = handle_api(
            user,
            project_id,
            "submit",
            json!({
                "body": {"task": "retired selector"},
                "method": "POST"
            }),
        )
        .expect_err("retired manifest ability field must be rejected");

        let msg = format!("{err:#}");
        assert!(
            msg.contains("unknown field `ability`") || msg.contains("ability_ura"),
            "error should point at retired ability field or replacement field: {msg}"
        );

        PUBLISHED_PROJECTS.remove(&key);
    }
}
