// EasyNet-Cli — Pages reference system: unit / integration tests
// =============================================================
//
// File: tests/pages_unit.rs
// Description: Matrix A from RFC-006-B v0.6 implementation plan.
//              In-process tests against the ability handlers
//              (no daemon, no IPC, no HTTP listener) plus the
//              kernel sandbox primitives.
//
// Cases:
//   U1   pages.publish of a two-file folder works
//   U2/3 fetch returns the bytes for hello-world.html / style.css
//   U4   path traversal /../../etc/passwd blocked at kernel layer
//   U5   nonexistent path returns Err
//   U6   dotfile probe (/.env) blocked by default-deny rule
//   U7   symlink escape blocked (RESOLVE_NO_SYMLINKS / O_NOFOLLOW)
//   U8   duplicate publish on same (user, project) rejected
//   U9   unpublish releases fd + removes registry entry
//   U10  list returns publish entries
//   U11  get returns one project's detail
//   U12  concurrent fetches succeed without fd leak
//   U13  file size cap enforced
//
// Conformance: RFC-006-B v0.6 §6.3, INV-1 / INV-2 / INV-3.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#[path = "key_service_fixture.rs"]
mod key_service_fixture;

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use easynet_axon::invocation::LocalRuntime;
use serde_json::{json, Value};

use easynet_cli::core::ura;
use easynet_cli::daemon::ability::builtins::resources::pages::fetch::handle_fetch;
use easynet_cli::daemon::ability::builtins::resources::pages::list_get_unpublish::{
    handle_get, handle_list, handle_unpublish, handle_unpublish_with_registry,
};
use easynet_cli::daemon::ability::builtins::resources::pages::publish::handle_publish;
use easynet_cli::daemon::ability::builtins::resources::pages::state::{
    DEFAULT_FILE_SIZE_CAP, PUBLISHED_PROJECTS,
};
use easynet_cli::daemon::ability::builtins::resources::pages::{self, PagesConfig};
use easynet_cli::daemon::ability::dispatch::{AbilityAuthorityContext, AxonAbilityCatalog};
use easynet_cli::daemon::invocation::routing::target::{CallMode, InvocationTarget, TargetScope};
use std::sync::Arc;

/// Per-test fixture: makes a temp folder with a unique project
/// id so concurrent test runs do not collide in the global
/// `PUBLISHED_PROJECTS` map.
struct TestHomeGuard {
    _lock: MutexGuard<'static, ()>,
    temp_dir: PathBuf,
    prev_home: Option<String>,
}

impl TestHomeGuard {
    fn new() -> Self {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let lock = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let temp_dir = std::env::temp_dir().join(format!(
            "easynet-pages-test-home-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::create_dir_all(&temp_dir);
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &temp_dir);
        Self {
            _lock: lock,
            temp_dir,
            prev_home,
        }
    }
}

impl Drop for TestHomeGuard {
    fn drop(&mut self) {
        match self.prev_home.take() {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
        let _ = fs::remove_dir_all(&self.temp_dir);
    }
}

struct Fixture {
    _home: TestHomeGuard,
    user: String,
    project_id: String,
    folder: PathBuf,
    realm: String,
    listener_port: u16,
    registry: Arc<AxonAbilityCatalog>,
}

fn registry_for_pages_owner(
    runtime: Arc<LocalRuntime>,
    realm: &str,
    user: &str,
) -> AxonAbilityCatalog {
    let pages_agent = ura::agent_ura(realm, user, "pages");
    let authority_context = AbilityAuthorityContext::for_combined_authority_roots(ura::device_ura(
        realm,
        "pages-test-device",
    ))
    .expect("Pages test Device authority context")
    .with_declared_agent_authority_root(pages_agent)
    .expect("Pages execution Agent must be explicitly hosted by the test daemon");
    AxonAbilityCatalog::new_with_runtime_and_authority_context(runtime, authority_context)
}

fn configured_local_runtime() -> Arc<LocalRuntime> {
    let runtime = LocalRuntime::new_local_fast();
    easynet_cli::daemon::axon_bridge::runtime_factory::configure_local_runtime(
        &runtime, None, None,
    );
    runtime
}

impl Fixture {
    fn new(name_seed: &str) -> Self {
        let home = TestHomeGuard::new();
        key_service_fixture::install();
        // unique project_id per call to dodge the duplicate-publish
        // rejection (U8 explicitly tests dup, others rely on
        // uniqueness for isolation)
        let pid = format!(
            "{name_seed}{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        );
        let folder = std::env::temp_dir().join(format!("easynet-pages-test-{pid}"));
        fs::create_dir_all(&folder).expect("create test folder");
        // Two stable files every test can fetch.
        fs::write(
            folder.join("hello-world.html"),
            "<!doctype html><h1>Hello, EasyNet.</h1>",
        )
        .expect("write hello");
        fs::write(folder.join("style.css"), "h1 { color: red; }").expect("write css");

        let user = format!("alice-{pid}");
        let realm = "easynet.run".to_string();
        let registry = Arc::new(registry_for_pages_owner(
            configured_local_runtime(),
            &realm,
            &user,
        ));

        Self {
            _home: home,
            user,
            project_id: pid,
            folder,
            realm,
            listener_port: 8787,
            registry,
        }
    }

    fn publish(&self) -> Value {
        let args = json!({
            "folder":     self.folder.display().to_string(),
            "project_id": self.project_id,
            "visibility": "public",
        });
        handle_publish(
            &self.user,
            self.listener_port,
            &self.realm,
            self.registry.clone(),
            args,
        )
        .expect("publish should succeed")
    }

    fn unpublish(&self) -> Value {
        handle_unpublish_with_registry(
            &self.user,
            self.registry.as_ref(),
            json!({ "project_id": self.project_id }),
        )
        .expect("unpublish should succeed")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // best-effort: remove from registry + delete temp dir
        let _ = handle_unpublish(&self.user, json!({ "project_id": self.project_id }));
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn fetch_bytes(user: &str, project_id: &str, path: &str) -> anyhow::Result<Vec<u8>> {
    let v = handle_fetch(user, project_id, json!({ "path": path }))?;
    let b64 = v
        .get("bytes_b64")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing bytes_b64"))?;
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| anyhow::anyhow!("b64 decode: {e}"))
}

fn local_rpc_target(ability: &str, args: Value) -> InvocationTarget {
    InvocationTarget {
        scope: TargetScope::Local,
        ability: ability.to_string(),
        normalized_args: args,
        call_mode: CallMode::Rpc,
        subject: None,
        causal_context: None,
        request_metadata: Default::default(),
    }
}

// ── U1 ──────────────────────────────────────────────────────────────
#[test]
fn u1_publish_two_file_folder() {
    let f = Fixture::new("u1");
    let result = f.publish();
    assert!(result.get("project_ura").is_some());
    assert!(result.get("url_root").is_some());
    let key = (f.user.clone(), f.project_id.clone());
    assert!(
        PUBLISHED_PROJECTS.contains_key(&key),
        "PUBLISHED_PROJECTS must contain the published key after publish"
    );
}

// ── U2 ──────────────────────────────────────────────────────────────
#[test]
fn u2_fetch_html() {
    let f = Fixture::new("u2");
    f.publish();
    let bytes = fetch_bytes(&f.user, &f.project_id, "/hello-world.html").expect("fetch html");
    assert!(
        std::str::from_utf8(&bytes)
            .unwrap()
            .contains("Hello, EasyNet"),
        "fetched bytes must equal the file on disk"
    );
}

// ── U3 ──────────────────────────────────────────────────────────────
#[test]
fn u3_fetch_css() {
    let f = Fixture::new("u3");
    f.publish();
    let bytes = fetch_bytes(&f.user, &f.project_id, "/style.css").expect("fetch css");
    assert_eq!(std::str::from_utf8(&bytes).unwrap(), "h1 { color: red; }");
    // MIME check via the raw value
    let v = handle_fetch(&f.user, &f.project_id, json!({"path": "/style.css"})).unwrap();
    assert_eq!(
        v.get("content_type").and_then(Value::as_str),
        Some("text/css; charset=utf-8")
    );
}

// ── U4 ──────────────────────────────────────────────────────────────
#[test]
fn u4_path_traversal_blocked() {
    let f = Fixture::new("u4");
    f.publish();
    // try the classic ../../etc/passwd
    let err = handle_fetch(
        &f.user,
        &f.project_id,
        json!({ "path": "/../../etc/passwd" }),
    )
    .expect_err("path traversal must fail");
    let msg = format!("{err}");
    // The macOS path joins root and the relative; the canonicalize
    // step then tries the full /etc/passwd which IS readable, but
    // the prefix-check refuses it. Linux refuses earlier in
    // openat2 with EXDEV. Both surface as some error string.
    assert!(
        msg.contains("escapes")
            || msg.contains("not found")
            || msg.contains("XDEV")
            || msg.contains("path"),
        "expected escape rejection, got: {msg}"
    );
}

// ── U5 ──────────────────────────────────────────────────────────────
#[test]
fn u5_nonexistent_returns_err() {
    let f = Fixture::new("u5");
    f.publish();
    let err = handle_fetch(
        &f.user,
        &f.project_id,
        json!({ "path": "/no-such-file.html" }),
    )
    .expect_err("nonexistent must error");
    assert!(format!("{err}").to_lowercase().contains("not found"));
}

// ── U6 ──────────────────────────────────────────────────────────────
#[test]
fn u6_dotfile_blocked() {
    let f = Fixture::new("u6");
    // even if the dotfile exists in the folder, the rule rejects.
    fs::write(f.folder.join(".env"), "secret_key=should_not_leak").unwrap();
    f.publish();
    let err = handle_fetch(&f.user, &f.project_id, json!({ "path": "/.env" }))
        .expect_err("dotfile must be refused");
    assert!(
        format!("{err}").contains("dotfile"),
        "expected dotfile message, got {err}"
    );
}

// ── U7 ──────────────────────────────────────────────────────────────
#[test]
fn u7_symlink_blocked() {
    let f = Fixture::new("u7");
    // Place a symlink inside the folder pointing at /etc/passwd.
    let link = f.folder.join("escape");
    let _ = std::os::unix::fs::symlink("/etc/passwd", &link);
    f.publish();
    let err = handle_fetch(&f.user, &f.project_id, json!({ "path": "/escape" }))
        .expect_err("symlink must be refused");
    let msg = format!("{err}");
    // A symlink whose target lives outside the root can be refused
    // either as "symlink/loop" (kernel sees and rejects the symlink
    // — Linux RESOLVE_NO_SYMLINKS path) OR as "escapes" (macOS
    // realpath resolves through the link and the prefix check
    // refuses the destination). Both are conformant with INV-3.
    assert!(
        msg.contains("symlink")
            || msg.contains("loop")
            || msg.contains("escapes")
            || msg.contains("not found"),
        "expected symlink/loop/escape rejection, got: {msg}"
    );
}

// ── U8 ──────────────────────────────────────────────────────────────
#[test]
fn u8_duplicate_publish_rejected() {
    let f = Fixture::new("u8");
    f.publish();
    // second call with same (user, project_id)
    let result = handle_publish(
        &f.user,
        f.listener_port,
        &f.realm,
        f.registry.clone(),
        json!({
            "folder":     f.folder.display().to_string(),
            "project_id": f.project_id,
            "visibility": "public",
        }),
    );
    let err = result.expect_err("duplicate must fail");
    assert!(format!("{err}").contains("already published"));
}

// ── U9 ──────────────────────────────────────────────────────────────
#[test]
fn u9_unpublish_clears_state() {
    let f = Fixture::new("u9");
    f.publish();
    let key = (f.user.clone(), f.project_id.clone());
    assert!(PUBLISHED_PROJECTS.contains_key(&key));
    let r = f.unpublish();
    assert_eq!(r.get("removed").and_then(Value::as_bool), Some(true));
    assert!(!PUBLISHED_PROJECTS.contains_key(&key));

    // Subsequent fetch must fail
    let err = handle_fetch(&f.user, &f.project_id, json!({"path": "/hello-world.html"}))
        .expect_err("post-unpublish fetch should fail");
    assert!(format!("{err}").contains("not published"));
}

// ── U10 ─────────────────────────────────────────────────────────────
#[test]
fn u10_list_returns_published() {
    let f = Fixture::new("u10");
    f.publish();
    let v =
        handle_list(&f.user, f.listener_port, &f.realm, json!({})).expect("list should succeed");
    let projects = v.get("projects").and_then(Value::as_array).unwrap();
    assert!(
        projects
            .iter()
            .any(|p| p.get("project_id").and_then(Value::as_str) == Some(f.project_id.as_str())),
        "list should include freshly-published project"
    );
}

// ── U11 ─────────────────────────────────────────────────────────────
#[test]
fn u11_get_returns_detail() {
    let f = Fixture::new("u11");
    f.publish();
    let v = handle_get(
        &f.user,
        f.listener_port,
        &f.realm,
        json!({ "project_id": f.project_id }),
    )
    .expect("get should succeed");
    assert_eq!(
        v.get("project_id").and_then(Value::as_str),
        Some(f.project_id.as_str())
    );
    assert!(v.get("project_ura").is_some());
    assert!(v.get("file_size_cap").is_some());
}

// ── U12 ─────────────────────────────────────────────────────────────
#[test]
fn u12_concurrent_fetches() {
    use std::thread;
    let f = Fixture::new("u12");
    f.publish();
    let user = f.user.clone();
    let pid = f.project_id.clone();

    let handles: Vec<_> = (0..32)
        .map(|i| {
            let user = user.clone();
            let pid = pid.clone();
            thread::spawn(move || {
                let path = if i % 2 == 0 {
                    "/hello-world.html"
                } else {
                    "/style.css"
                };
                let v =
                    handle_fetch(&user, &pid, json!({ "path": path })).expect("concurrent fetch");
                assert!(v.get("bytes_b64").is_some());
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread join");
    }
}

// ── U13 ─────────────────────────────────────────────────────────────
#[test]
fn u13_file_size_cap_enforced() {
    let f = Fixture::new("u13");
    // A sparse file exercises the real metadata size gate without allocating
    // or writing 100 MiB in the test process.
    let oversized = fs::File::create(f.folder.join("oversized.bin"))
        .expect("create oversized sparse test file");
    oversized
        .set_len(DEFAULT_FILE_SIZE_CAP + 1)
        .expect("set sparse test file length");
    f.publish();
    let err = handle_fetch(&f.user, &f.project_id, json!({"path": "/oversized.bin"}))
        .expect_err("over-cap fetch should fail");
    assert!(format!("{err}").contains("size") || format!("{err}").contains("cap"));
}

#[test]
fn u14_pages_management_abilities_are_in_local_runtime() {
    let _home = TestHomeGuard::new();
    key_service_fixture::install();
    let runtime = configured_local_runtime();
    let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
    let user = "alice-runtime";
    let pages_agent = ura::agent_ura("easynet.run", user, "pages");
    let mut reg = registry_for_pages_owner(Arc::clone(&runtime), "easynet.run", user);

    pages::register(
        &mut reg,
        PagesConfig {
            user: user.to_string(),
            realm: "easynet.run".to_string(),
            listener_port: 8787,
        },
        Arc::clone(&handle),
    );
    let reg = Arc::new(reg);
    assert!(handle.set(Arc::clone(&reg)).is_ok());

    // Backend routes `project_list` through the synthetic hosted agent
    // `agent/<user>.pages`. The local dispatch key stays owner-qualified, but
    // the control-plane authority root must be the pages agent, not the
    // user's account-agent fallback.
    let ability = ura::local_dispatch_ability_key(&pages_agent, "project_list");
    assert_eq!(ability, "pages.project_list");
    assert!(reg.has_rpc(&ability));
    let facts = reg
        .runtime_binding_facts_for_mode(&ability, easynet_cli::daemon::ability::CallMode::Rpc)
        .expect("runtime facts lookup")
        .expect("project_list runtime facts");
    assert_eq!(facts.authority_owner_projection, "user:alice-runtime");
    assert_eq!(facts.authority_root, pages_agent);
    let public_name = ura::owner_local_ability_name(&facts.authority_root, "project_list");
    let runtime_ability = ura::owner_ability_ura(&facts.authority_root, &public_name)
        .expect("project_list runtime ability URA");
    assert_eq!(
        runtime_ability,
        "easynet:///r/easynet.run/ability/alice-runtime.pages.project_list"
    );
    assert!(
        futures::executor::block_on(runtime.has_ability(&runtime_ability)),
        "project_list must be installed in Axon LocalRuntime under the same canonical ability URA the carrier dispatch path probes"
    );
    let resp = reg
        .invoke_rpc_target_json(local_rpc_target(&ability, json!({})))
        .expect("project_list should invoke through LocalRuntime");
    assert_eq!(resp["projects"].as_array().map(Vec::len), Some(0));
}

#[test]
fn u15_publish_registers_project_abilities_in_local_runtime() {
    let f = Fixture::new("u15");
    fs::create_dir_all(f.folder.join("api")).expect("api dir");
    fs::write(
        f.folder.join("api/ping.toml"),
        "kind = \"static_json\"\n[response]\npong = true\n",
    )
    .expect("write api manifest");

    f.publish();
    let fetch_ability = format!("{}.{}.page.fetch", f.user, f.project_id);
    let api_ability = format!("{}.{}.api.ping", f.user, f.project_id);
    assert!(f.registry.has_rpc(&fetch_ability));
    assert!(f.registry.has_rpc(&api_ability));

    let fetched = f
        .registry
        .invoke_rpc_target_json(local_rpc_target(
            &fetch_ability,
            json!({"path": "/hello-world.html"}),
        ))
        .expect("fetch should invoke through LocalRuntime");
    assert!(fetched.get("bytes_b64").is_some());

    let api_resp = f
        .registry
        .invoke_rpc_target_json(local_rpc_target(
            &api_ability,
            json!({"body": {}, "method": "GET"}),
        ))
        .expect("api should invoke through LocalRuntime");
    assert_eq!(api_resp["body"]["pong"], true);

    f.unpublish();
    assert!(!f.registry.has_rpc(&fetch_ability));
    assert!(!f.registry.has_rpc(&api_ability));
}
