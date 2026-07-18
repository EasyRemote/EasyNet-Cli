//! Tests for the `easynet agent` command family (moved verbatim
//! from cli/agent.rs, F-033 / T4.6).

use super::*;

use crate::daemon::execution::mission::directory::AgentDirectory;
use crate::daemon::persistence::config;

#[test]
fn eal_string_literal_quotes_and_escapes_metachars() {
    // Round-trip property: every char that would either terminate
    // the literal or change the lexer's byte-skipping behaviour
    // must be escaped. The EAL lexer is intentionally non-decoding
    // (it only uses `\` to skip the next byte), so we don't need
    // numeric `\uXXXX` escapes — just the quote/backslash pair plus
    // the readability escapes for newline/tab.
    assert_eq!(eal_string_literal("hello").unwrap(), "\"hello\"");
    assert_eq!(eal_string_literal(r#"a"b"#).unwrap(), r#""a\"b""#);
    assert_eq!(eal_string_literal(r"a\b").unwrap(), r#""a\\b""#);
    assert_eq!(eal_string_literal("a\nb").unwrap(), r#""a\nb""#);
}

#[test]
fn eal_string_literal_rejects_embedded_nul() {
    // Downstream agent CLIs treat the prompt as a C string and
    // silently truncate at the first NUL. Surface the bad input as
    // an error at the CLI layer rather than delivering a corrupt
    // half-prompt to the model.
    let err = eal_string_literal("good\0bad").unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("NUL"),
        "expected NUL-rejection error, got: {msg}"
    );
}

// ── v2 CLI verbs ────────────────────────────────────────────────────

use crate::cli::commands::test_support::HomeGuard;
use crate::core::agent::spec::{AgentSpec, RuntimeKind};
use crate::daemon::execution::mission::directory::Location;
use crate::daemon::persistence::agent_registry as agents;
use crate::daemon::persistence::agent_registry::CURRENT_REGISTRY_SCHEMA;
use std::fs;
use std::sync::{Arc, OnceLock};

#[derive(Debug)]
struct AgentCommandFixtureGateway;

impl AgentCommandGateway for AgentCommandFixtureGateway {
    fn invoke(&self, ability: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        invoke_agent_command_fixture(ability, args)
    }
}

fn invoke_agent_command_fixture(
    ability: &str,
    args: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
        crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
        None,
    );
    let dispatch_handle = Arc::new(OnceLock::new());
    let registrar = crate::daemon::axon_bridge::hot_agent_registrar::HotAgentRegistrar::new_pending(
        Arc::new(Vec::new()),
        Arc::clone(&dispatch_handle),
        Arc::new(
            crate::daemon::ability::builtins::agents::discover::DetachedDiscoverFederationResolver,
        ),
    );
    registrar
        .set_runtime(Arc::clone(&runtime))
        .map_err(|error| anyhow::anyhow!("wire Agent command fixture runtime: {error}"))?;

    let hot_registrar: Arc<
        crate::daemon::ability::builtins::agents::lifecycle::SharedHotRegistrarCell,
    > = Arc::new(OnceLock::new());
    hot_registrar
        .set(registrar)
        .map_err(|_| anyhow::anyhow!("wire Agent command fixture registrar twice"))?;

    let authority_context =
        crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
            crate::core::ura::device_ura("localhost", "dev-1"),
            Vec::<String>::new(),
        )
        .map_err(|error| anyhow::anyhow!("build Agent command fixture authority: {error}"))?;
    let mut catalog = crate::daemon::ability::dispatch::AxonAbilityCatalog::new_with_runtime_and_authority_context(
        runtime,
        authority_context,
    );
    crate::daemon::ability::builtins::agents::list::register(&mut catalog, || {
        crate::daemon::persistence::agent_aggregate::AgentAggregateRepository::load_snapshot()
            .map_err(|error| anyhow::anyhow!("fixture agent.list aggregate load: {error:#}"))
    });
    crate::daemon::ability::builtins::agents::lifecycle::register(
        &mut catalog,
        Arc::clone(&hot_registrar),
    );
    let meta_catalog_handle = Arc::clone(&dispatch_handle);
    catalog.register_rpc_with_owner(
        "meta.list_abilities",
        crate::daemon::ability::dispatch::OwnerKind::Device,
        Arc::new(move |args: serde_json::Value| {
            let owner_ura = args
                .get("agent_ura")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("fixture meta.list_abilities requires agent_ura"))?;
            let catalog = meta_catalog_handle
                .get()
                .ok_or_else(|| anyhow::anyhow!("fixture catalog handle is not initialized"))?;
            let publication =
                crate::daemon::ability::catalog::LocalAbilityPublicationSnapshot::capture(
                    catalog.as_ref(),
                );
            let abilities = publication
                .owner_descriptors(owner_ura)
                .into_iter()
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(serde_json::json!({"abilities": abilities}))
        }),
    );
    let catalog = Arc::new(catalog);
    dispatch_handle
        .set(Arc::clone(&catalog))
        .map_err(|_| anyhow::anyhow!("wire Agent command fixture dispatch handle twice"))?;
    if ability == "meta.list_abilities" {
        let registrar = hot_registrar
            .get()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("fixture registrar is not initialized"))?;
        let rows = agents::load_agents()?;
        crate::daemon::axon_bridge::hot_agent_registrar::block_on_hot_registrar(async move {
            for (name, entry) in rows.agents {
                registrar.register_agent(&name, &entry).await?;
            }
            Ok::<(), crate::daemon::axon_bridge::hot_agent_registrar::HotAgentRegistrarError>(())
        })?;
    }
    if let Some(handler) = catalog.resolve_rpc_with_env(ability) {
        let device_ura = crate::core::ura::device_ura("localhost", "dev-1");
        return handler(
            crate::daemon::ability::dispatch::EnvelopeContext::for_test_targeted_ability(
                crate::core::ura::LOCAL_SYSTEM_AGENT_URA,
                &device_ura,
                ability,
                &device_ura,
            ),
            args,
        );
    }
    catalog.invoke_rpc_target_json(
        crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root(
            ability,
            args,
            crate::daemon::invocation::routing::target::CallMode::Rpc,
        ),
    )
}

struct JoinedHome {
    _home: HomeGuard,
    _gateway: crate::cli::daemon_client::agent_gateway::TestAgentCommandGatewayGuard,
}

/// Build the AddArgs shape the CLI surface would construct
/// for `easynet agent add <name> --type <t> --model <m>`.
/// We don't drive clap here — we exercise the `run_add`
/// body directly, which is the contract-bearing surface.
fn add_args(name: &str, r#type: &str, model: Option<&str>) -> AddArgs {
    AddArgs {
        name: name.into(),
        r#type: r#type.into(),
        model: model.map(str::to_string),
        label: None,
        command: None,
        args: Vec::new(),
    }
}

fn seed_joined_credentials() {
    crate::daemon::persistence::config::save_credentials(
        &crate::daemon::persistence::config::Credentials {
            node_id: "dev-1".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: "axon://hub.test:50051".to_string(),
            realm: "localhost".to_string(),
            username: Some("dev".to_string()),
            user_id: Some("user-dev".to_string()),
            ..Default::default()
        },
    )
    .expect("seed joined credentials");
}

fn joined_home() -> JoinedHome {
    let home = HomeGuard::new();
    seed_joined_credentials();
    let gateway = crate::cli::daemon_client::agent_gateway::install_test_agent_command_gateway(
        Arc::new(AgentCommandFixtureGateway),
    );
    JoinedHome {
        _home: home,
        _gateway: gateway,
    }
}

#[cfg(unix)]
fn write_cli_mcp_echo_server(dir: &std::path::Path) -> std::path::PathBuf {
    let script = dir.join("echo_mcp.sh");
    fs::write(
            &script,
            r#"#!/bin/sh
exec python3 -u -c '
import sys, json
def read_msg():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.decode().strip()
        if not line:
            break
        name, value = line.split(":", 1)
        headers[name.lower()] = value.strip()
    body = sys.stdin.buffer.read(int(headers["content-length"]))
    return json.loads(body)
def write_msg(resp):
    body = json.dumps(resp).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()
while True:
    req = read_msg()
    if req is None:
        break
    rid = req.get("id")
    method = req.get("method")
    if method == "initialize":
        result = {"protocolVersion": "2024-11-05", "capabilities": {}, "serverInfo": {"name": "echo", "version": "0"}}
    elif method == "tools/list":
        result = {"tools": [
            {"name": "echo-text", "description": "Echo text through MCP", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}}
        ]}
    else:
        result = {"content": [{"type": "text", "text": "ok"}], "isError": False}
    write_msg({"jsonrpc": "2.0", "id": rid, "result": result})
'
"#,
        )
        .expect("write echo mcp");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
    script
}

#[cfg(unix)]
#[test]
fn run_mcp_add_writes_mcp_exec_manifest_for_agent() {
    let _home = joined_home();
    run_add(add_args("codex", "codex", None)).expect("agent add");
    let tmp = tempfile::tempdir().expect("tempdir");
    let server = write_cli_mcp_echo_server(tmp.path());
    let mcp_dir = crate::daemon::persistence::config::state_dir();
    fs::create_dir_all(&mcp_dir).expect("state dir");
    let config_path = mcp_dir.join("mcp_clients.json");
    fs::write(
        &config_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "servers": [{
                "name": "Echo Server",
                "command": server.display().to_string(),
                "args": [],
                "stdio_framing": "content-length"
            }]
        }))
        .unwrap(),
    )
    .expect("write mcp config");

    run_mcp_add(McpAddArgs {
        name: "codex".into(),
        server: Some("Echo Server".into()),
        tools: vec![],
        prefix: "mcp".into(),
        config: Some(config_path),
        dry_run: false,
        overwrite: false,
        skip_unreachable: false,
        cost_kind: None,
        cost_label: None,
    })
    .expect("mcp add");

    let manifest_path = crate::daemon::persistence::config::agents_root()
        .join("codex")
        .join("abilities")
        .join("mcp_echo_server_echo_text.ability.toml");
    let body = fs::read_to_string(&manifest_path).expect("manifest written");
    let manifest =
        crate::daemon::ability::manifest::AbilityManifest::from_toml_str(&body).expect("parse");
    assert_eq!(manifest.name(), "mcp_echo_server_echo_text");
    match manifest.exec().expect("exec") {
        crate::daemon::ability::manifest::AbilityExec::Mcp(exec) => {
            assert_eq!(exec.server, "Echo Server");
            assert_eq!(exec.tool, "echo-text");
        }
        other => panic!("expected mcp exec, got {other:?}"),
    }
    assert_eq!(
        manifest.input_schema()["properties"]["text"]["type"],
        serde_json::Value::String("string".into())
    );
}

#[test]
fn scoped_script_add_commits_manifest_through_daemon_authoring_transaction() {
    let _home = joined_home();
    run_add(add_args("codex", "codex", None)).expect("agent add");

    crate::cli::commands::agent_new_ability::run_scoped(
        "codex",
        &[
            "new-ability".to_string(),
            "script".to_string(),
            "add".to_string(),
            "echo_value".to_string(),
            "--".to_string(),
            "printf".to_string(),
            "{{ value }}".to_string(),
        ],
    )
    .expect("script ability authoring transaction");

    let path = config::agents_root()
        .join("codex")
        .join("abilities")
        .join("echo_value.ability.toml");
    let body = fs::read_to_string(path).expect("daemon committed manifest");
    let manifest = crate::daemon::ability::manifest::AbilityManifest::from_toml_str(&body)
        .expect("committed manifest parses");
    assert_eq!(manifest.name(), "echo_value");
    assert!(manifest.exec().is_some());
}

#[test]
fn daemon_authoring_response_proves_custom_ability_is_in_live_publication() {
    let _home = joined_home();
    run_add(add_args("codex", "codex", None)).expect("agent add");
    let manifest = crate::daemon::ability::manifest::AbilityManifest::new(
        "echo_live",
        "Echo one value",
        serde_json::json!({
            "type": "object",
            "properties": {"value": {"type": "string"}}
        }),
    )
    .unwrap()
    .with_exec(crate::daemon::ability::manifest::AbilityExec::Shell(
        crate::daemon::ability::manifest::ShellExec {
            argv: vec!["printf".to_string(), "{{ value }}".to_string()],
            stdout: None,
            sandbox: None,
        },
    ))
    .unwrap();
    let response = agent_command_gateway()
        .invoke(
            crate::daemon::ability::builtins::agents::authoring::ABILITY_PUT_AGENT_ABILITY,
            serde_json::json!({
                "name": "codex",
                "manifests_toml": [manifest.to_toml_string().unwrap()],
                "overwrite": false,
                "conflict_policy": "reject"
            }),
        )
        .expect("daemon authoring transaction");

    assert_eq!(response["state"], "committed");
    assert!(response["publication"]
        .as_array()
        .unwrap()
        .iter()
        .any(|descriptor| descriptor["name"] == "echo_live"));
}

#[test]
fn normalize_mcp_input_schema_removes_toml_unsupported_nulls() {
    let normalized = normalize_mcp_input_schema(Some(serde_json::json!({
        "type": "object",
        "properties": {
            "q": {"type": ["string", null], "default": null}
        }
    })));
    let manifest = crate::daemon::ability::manifest::AbilityManifest::new(
        "mcp_null_schema",
        "schema with upstream nulls",
        normalized,
    )
    .expect("schema should validate");
    manifest
        .to_toml_string()
        .expect("normalized schema should serialize to TOML");
}

#[test]
fn run_add_writes_v2_row_and_materializes_agent_directory() {
    // Fresh add must: (a) insert a v2 registry row
    // carrying `root_path` + `schema_version=2`; (b)
    // create the agent directory on disk with an
    // `agent.toml` that reflects the CLI flags; (c) leave
    // the fat fields (`command`, `args`) empty so they
    // omit on serialize — the whole point of the CLI
    // rewrite is that v2 rows do not carry vestigial v1
    // data.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", Some("claude-opus-4-7"))).unwrap();

    let registry = agents::load_agents().unwrap();
    let alice = registry.agents.get("alice").expect("alice registered");
    assert_eq!(alice.schema_version, CURRENT_REGISTRY_SCHEMA);
    assert!(alice.root_path.is_some());
    // Fat-field cleanliness: fresh v2 row must not carry
    // command / args from `AgentEntry::new`.
    assert!(alice.command.is_empty());
    assert!(alice.args.is_empty());

    // Directory materialized with a real agent.toml that
    // reflects the CLI flags.
    let root = alice.root_path.as_ref().unwrap();
    let toml = fs::read_to_string(root.join("agent.toml")).unwrap();
    let spec = AgentSpec::from_toml_str(&toml).unwrap();
    assert_eq!(spec.name, "alice");
    assert_eq!(spec.runtime, RuntimeKind::ClaudeCode);
    assert_eq!(spec.model.as_deref(), Some("claude-opus-4-7"));
}

#[test]
fn run_add_update_preserves_operator_edits_to_agent_toml() {
    // Repeat `agent add` with a different model must
    // update the registry row (new model reflected in
    // `AgentEntry.model`) but NOT clobber a hand-written
    // `description` in agent.toml. The contract: CLI
    // flags update the registry-visible subset; operator
    // edits to agent.toml survive.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", Some("old-model"))).unwrap();

    let registry = agents::load_agents().unwrap();
    let root = registry.agents["alice"].root_path.as_ref().unwrap().clone();

    // Hand-edit agent.toml to add a description.
    let mut spec =
        AgentSpec::from_toml_str(&fs::read_to_string(root.join("agent.toml")).unwrap()).unwrap();
    spec.description = Some("user-edited".into());
    fs::write(root.join("agent.toml"), spec.to_toml_string().unwrap()).unwrap();

    // Re-run agent add with a different model.
    run_add(add_args("alice", "claude-code", Some("new-model"))).unwrap();

    // The operator's description must survive; we do not
    // rewrite agent.toml on update, we only update the
    // registry row.
    let spec2 =
        AgentSpec::from_toml_str(&fs::read_to_string(root.join("agent.toml")).unwrap()).unwrap();
    assert_eq!(spec2.description.as_deref(), Some("user-edited"));
}

#[test]
fn run_remove_default_keeps_the_on_disk_root() {
    // `agent remove` without --purge must strip the
    // registry row but leave the directory (and its
    // `.env`, `runs/`, operator edits) intact. The rule
    // is "default to non-destructive"; credentials are at
    // stake and a second `agent add` on the same name can
    // legitimately want the old history back.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", None)).unwrap();
    let root = agents::load_agents().unwrap().agents["alice"]
        .root_path
        .clone()
        .unwrap();
    assert!(root.join("agent.toml").exists());

    run_remove(RemoveArgs {
        name: "alice".into(),
        purge: false,
    })
    .unwrap();

    // Registry row gone.
    assert!(!agents::load_agents().unwrap().agents.contains_key("alice"));
    // Directory still present.
    assert!(
        root.join("agent.toml").exists(),
        "--purge not passed: dir must stay"
    );
}

#[test]
fn run_remove_with_purge_deletes_the_on_disk_root() {
    // `agent remove --purge` deletes the directory too.
    // This is the explicit destructive path.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", None)).unwrap();
    let root = agents::load_agents().unwrap().agents["alice"]
        .root_path
        .clone()
        .unwrap();

    run_remove(RemoveArgs {
        name: "alice".into(),
        purge: true,
    })
    .unwrap();

    assert!(
        !root.exists(),
        "--purge must delete the directory, but {} still exists",
        root.display()
    );
}

#[derive(Default)]
struct RecordingRemovalGateway {
    calls: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
}

impl AgentCommandGateway for RecordingRemovalGateway {
    fn invoke(&self, ability: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        self.calls.lock().unwrap().push((ability.to_string(), args));
        match ability {
            "agent.stop" => Ok(serde_json::json!({
                "ack": true,
                "runtime_removed": 0,
                "removed_entry": {"root_path": "/tmp/kept-agent-root"},
            })),
            "agent.purge" => Ok(serde_json::json!({
                "ack": true,
                "runtime_removed": 0,
                "purge_state": "purged",
                "purged_path": "/tmp/purged-agent-root",
            })),
            other => anyhow::bail!("unexpected ability {other}"),
        }
    }
}

#[test]
fn remove_routes_non_destructive_and_destructive_authority_separately() {
    let _home = HomeGuard::new();
    let gateway = Arc::new(RecordingRemovalGateway::default());
    let _gateway_guard =
        crate::cli::daemon_client::agent_gateway::install_test_agent_command_gateway(
            gateway.clone(),
        );

    run_remove(RemoveArgs {
        name: "kept".into(),
        purge: false,
    })
    .unwrap();
    run_remove(RemoveArgs {
        name: "destroyed".into(),
        purge: true,
    })
    .unwrap();

    let calls = gateway.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0],
        (
            "agent.stop".to_string(),
            serde_json::json!({"name": "kept"})
        )
    );
    assert_eq!(
        calls[1],
        (
            "agent.purge".to_string(),
            serde_json::json!({"name": "destroyed"})
        )
    );
}

fn set_args(name: &str, model: Option<&str>) -> SetArgs {
    SetArgs {
        name: name.into(),
        model: model.map(str::to_string),
    }
}

#[derive(Default)]
struct MissingRootAgentGateway {
    calls: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
}

impl AgentCommandGateway for MissingRootAgentGateway {
    fn invoke(&self, ability: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        self.calls.lock().unwrap().push((ability.to_string(), args));
        match ability {
            "agent.list" => Ok(serde_json::json!({
                "agents": [{
                    "name": "alice",
                    "runtime": "claude-code",
                    "model": "sonnet",
                    "root_exists": null
                }]
            })),
            other => anyhow::bail!("unexpected daemon mutation `{other}`"),
        }
    }
}

#[test]
fn run_set_changes_model_in_both_agent_toml_and_registry_row() {
    // The on-disk `agent.toml` and the registry row must agree
    // after `agent set --model X`. Earlier versions only
    // updated one; the discrepancy showed up later as
    // "claude reports sonnet, but `agent list` shows opus" —
    // the contract here pins both.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", Some("sonnet"))).unwrap();

    run_set(set_args("alice", Some("opus"))).unwrap();

    // Registry row updated.
    let entry = agents::load_agents().unwrap().agents["alice"].clone();
    assert_eq!(entry.model.as_deref(), Some("opus"));

    // agent.toml on disk updated.
    let root = entry.root_path.clone().unwrap();
    let spec =
        AgentSpec::from_toml_str(&fs::read_to_string(root.join("agent.toml")).unwrap()).unwrap();
    assert_eq!(spec.model.as_deref(), Some("opus"));
}

#[test]
fn run_set_preserves_project_local_root_path() {
    // `agent set` is now a daemon ability invoke. The CLI must
    // still preserve an existing registry row's custom root_path;
    // otherwise project-local agents get silently rewritten into
    // the global agents root during a model update.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", Some("sonnet"))).unwrap();

    let custom_root = crate::daemon::persistence::config::home_dir()
        .join("project")
        .join("agents")
        .join("alice");
    let mut spec = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
    spec.model = Some("sonnet".to_string());
    AgentDirectory::create(
        &Location::Local {
            root: custom_root.clone(),
        },
        spec,
    )
    .unwrap();

    let mut registry = agents::load_agents().unwrap();
    registry.agents.get_mut("alice").unwrap().root_path = Some(custom_root.clone());
    agents::save_agents(&registry).unwrap();

    run_set(set_args("alice", Some("opus"))).unwrap();

    let entry = agents::load_agents().unwrap().agents["alice"].clone();
    let canonical_root = std::fs::canonicalize(&custom_root).unwrap();
    assert_eq!(entry.root_path.as_deref(), Some(canonical_root.as_path()));
    let spec =
        AgentSpec::from_toml_str(&fs::read_to_string(custom_root.join("agent.toml")).unwrap())
            .unwrap();
    assert_eq!(spec.model.as_deref(), Some("opus"));
}

#[test]
fn run_set_with_empty_model_string_clears_the_field() {
    // Passing `--model ''` is the explicit CLEAR signal:
    // the agent should fall back to the underlying CLI's
    // default model. This is the load-bearing distinction
    // between "no flag passed" (no change) and "flag with
    // empty value" (clear).
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", Some("sonnet"))).unwrap();

    run_set(set_args("alice", Some(""))).unwrap();

    let entry = agents::load_agents().unwrap().agents["alice"].clone();
    assert!(
        entry.model.is_none(),
        "empty-string --model must clear; got {:?}",
        entry.model
    );
    // agent.toml round-trips with no `model` field.
    let root = entry.root_path.clone().unwrap();
    let body = fs::read_to_string(root.join("agent.toml")).unwrap();
    assert!(
        !body.contains("model ="),
        "cleared model must not be persisted; got:\n{body}"
    );
}

#[test]
fn run_set_rejects_unknown_agent_with_actionable_message() {
    // No false positives — `agent set nonexistent --model X`
    // must fail with a clear message pointing at `agent list`,
    // not silently create a row (which would be a footgun:
    // operator typos a name and gets a phantom agent).
    let _g = joined_home();
    let err = run_set(set_args("nonexistent", Some("sonnet"))).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("not registered"), "msg: {msg}");
    assert!(msg.contains("agent list"), "msg should hint list: {msg}");
}

#[test]
fn run_set_missing_root_path_fails_before_agent_start_is_sent() {
    let _home = HomeGuard::new();
    let gateway = Arc::new(MissingRootAgentGateway::default());
    let _gateway_guard =
        crate::cli::daemon_client::agent_gateway::install_test_agent_command_gateway(
            gateway.clone(),
        );

    let error = run_set(set_args("alice", Some("opus")))
        .expect_err("missing daemon root projection must block set");

    assert!(error.to_string().contains("omitted root_path"));
    let calls = gateway.calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "agent.start must not be sent");
    assert_eq!(calls[0].0, "agent.list");
}

#[test]
fn run_set_with_no_flags_errors_explicitly() {
    // `agent set alice` (no --model) is meaningless today.
    // We could silently no-op, but that risks operators
    // believing they changed something when they didn't.
    // Explicit error is friendlier.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", Some("sonnet"))).unwrap();
    let err = run_set(set_args("alice", None)).unwrap_err();
    assert!(format!("{err}").contains("nothing to change"));
}

#[test]
fn run_set_does_not_validate_model_string_against_any_allow_list() {
    // Per the SetArgs::model doc: claude/codex CLIs accept any
    // string and resolve aliases at their own discretion. Even
    // a deliberately-wrong-looking name must round-trip — the
    // validation belongs at invocation time, not at
    // configuration time. This pins the no-allow-list policy.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", None)).unwrap();
    run_set(set_args("alice", Some("definitely-not-a-real-model-xyz"))).unwrap();
    let entry = agents::load_agents().unwrap().agents["alice"].clone();
    assert_eq!(
        entry.model.as_deref(),
        Some("definitely-not-a-real-model-xyz")
    );
}

#[test]
fn run_prune_removes_orphaned_rows_only() {
    // With two agents — one whose root exists, one whose
    // root has been deleted — `prune` must remove only
    // the orphan. The surviving one must stay, and both
    // its directory and its row must be intact.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", None)).unwrap();
    run_add(add_args("bob", "codex", None)).unwrap();

    // Orphan bob by deleting its root.
    let bob_root = agents::load_agents().unwrap().agents["bob"]
        .root_path
        .clone()
        .unwrap();
    fs::remove_dir_all(&bob_root).unwrap();

    run_prune(PruneArgs { dry_run: false }).unwrap();

    let registry = agents::load_agents().unwrap();
    assert!(registry.agents.contains_key("alice"), "alice must survive");
    assert!(!registry.agents.contains_key("bob"), "bob must be pruned");
}

#[test]
fn run_prune_dry_run_leaves_registry_unchanged() {
    // The `--dry-run` contract is "no mutations". Rows
    // reported as "would prune" must still be present in
    // the registry after the command returns.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", None)).unwrap();
    let root = agents::load_agents().unwrap().agents["alice"]
        .root_path
        .clone()
        .unwrap();
    fs::remove_dir_all(&root).unwrap();

    run_prune(PruneArgs { dry_run: true }).unwrap();

    // Row must still be present — dry-run MUST NOT
    // mutate the registry. This is the load-bearing
    // property that makes `prune --dry-run` safe to run
    // as a recon step.
    assert!(agents::load_agents().unwrap().agents.contains_key("alice"));
}

// ── abilities / publish dry-run ─────────────────────────────────────

#[test]
fn run_abilities_lists_the_seeded_chat_manifest_for_a_fresh_agent() {
    // Fresh `agent add` always ships a default chat manifest.
    // `agent abilities` must surface it exactly once with its
    // fully-qualified `<agent>.chat` name.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", None)).unwrap();
    // Happy path is "no error"; we leave the eprintln-based
    // output un-asserted (tested at helper level via
    // list_ability_manifests).
    run_abilities(AbilitiesArgs {
        name: "alice".into(),
    })
    .expect("fresh agent must list its seeded chat manifest");
}

#[test]
fn run_abilities_reports_the_unknown_agent_as_an_error() {
    // `agent abilities <unknown>` must fail loud — we do not
    // want the empty-list path to mask a typo'd agent name.
    let _g = joined_home();
    let err = run_abilities(AbilitiesArgs {
        name: "nobody".into(),
    })
    .expect_err("unknown agent must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("nobody"),
        "error must name the missing agent: {msg}"
    );
    assert!(
        msg.contains("not registered") || msg.contains("add"),
        "error must hint at remediation: {msg}"
    );
}

#[test]
fn run_abilities_reports_missing_root_as_an_error() {
    // A row whose root was `rm -rf`d must not silently fall
    // through to "empty abilities list" — the operator needs
    // to see the true cause.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", None)).unwrap();
    let root = agents::load_agents().unwrap().agents["alice"]
        .root_path
        .clone()
        .unwrap();
    fs::remove_dir_all(&root).unwrap();
    let err = run_abilities(AbilitiesArgs {
        name: "alice".into(),
    })
    .expect_err("orphan row must error on 'agent abilities'");
    assert!(format!("{err}").contains("no on-disk root"));
}

#[test]
fn run_abilities_handles_empty_abilities_directory_without_error() {
    // An operator can legitimately remove every manifest to
    // hide the agent from discovery. That must succeed with
    // no panic, no error — just an empty-list signal.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", None)).unwrap();
    let root = agents::load_agents().unwrap().agents["alice"]
        .root_path
        .clone()
        .unwrap();
    // Wipe the seeded default.
    fs::remove_dir_all(root.join("abilities")).unwrap();
    fs::create_dir_all(root.join("abilities")).unwrap();
    run_abilities(AbilitiesArgs {
        name: "alice".into(),
    })
    .expect("empty abilities dir must be non-fatal");
}

#[test]
fn run_abilities_surfaces_manifest_parse_errors() {
    // A malformed manifest must surface as an error — silent
    // skip would hide it from the operator reviewing their
    // ability set before a publish.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", None)).unwrap();
    let root = agents::load_agents().unwrap().agents["alice"]
        .root_path
        .clone()
        .unwrap();
    fs::write(
        root.join("abilities").join("bad.ability.toml"),
        "not = valid = toml",
    )
    .unwrap();
    let err = run_abilities(AbilitiesArgs {
        name: "alice".into(),
    })
    .expect_err("malformed manifest must surface");
    assert!(format!("{err}").contains("bad"));
}

#[test]
fn run_publish_dry_run_succeeds_on_a_fresh_agent() {
    // The whole point of PR-4: dry-run shows what a future
    // publish would register without calling Axon. It must
    // succeed on a freshly-added agent (which has exactly
    // one seeded chat manifest).
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", None)).unwrap();
    run_publish(PublishArgs {
        name: "alice".into(),
        dry_run: true,
    })
    .expect("dry-run must succeed on a fresh agent");
}

#[test]
fn run_publish_reconciles_and_projects_live_publication() {
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", None)).unwrap();
    run_publish(PublishArgs {
        name: "alice".into(),
        dry_run: false,
    })
    .expect("non-dry-run must reconcile the daemon live publication");
}

#[test]
fn run_publish_reports_unknown_agent_before_checking_flags() {
    // An unknown agent name is a different error than
    // "flag not set". The unknown-agent check happens even
    // when --dry-run is passed, so the operator sees the
    // most-specific error first.
    let _g = joined_home();
    let err = run_publish(PublishArgs {
        name: "nobody".into(),
        dry_run: true,
    })
    .expect_err("unknown agent must error");
    assert!(format!("{err}").contains("nobody"));
}

#[test]
fn run_publish_dry_run_reads_live_baseline_after_manifest_directory_is_empty() {
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", None)).unwrap();
    let root = agents::load_agents().unwrap().agents["alice"]
        .root_path
        .clone()
        .unwrap();
    fs::remove_dir_all(root.join("abilities")).unwrap();
    fs::create_dir_all(root.join("abilities")).unwrap();
    run_publish(PublishArgs {
        name: "alice".into(),
        dry_run: true,
    })
    .expect("dry-run must project the live hosted-Agent baseline");
}

#[test]
fn run_publish_dry_run_does_not_mutate_registry_or_filesystem() {
    // Pinning the "no mutation" contract. If a future
    // refactor accidentally made dry-run touch state, this
    // test would catch it — compare registry bytes and the
    // abilities directory modtime before/after.
    let _g = joined_home();
    run_add(add_args("alice", "claude-code", None)).unwrap();
    let root = agents::load_agents().unwrap().agents["alice"]
        .root_path
        .clone()
        .unwrap();

    let registry_path = config::state_dir().join("agents.json");
    let before_registry = fs::read(&registry_path).unwrap();
    let before_ability = fs::read(root.join("abilities").join("chat.ability.toml")).unwrap();

    run_publish(PublishArgs {
        name: "alice".into(),
        dry_run: true,
    })
    .unwrap();

    let after_registry = fs::read(&registry_path).unwrap();
    let after_ability = fs::read(root.join("abilities").join("chat.ability.toml")).unwrap();
    assert_eq!(
        before_registry, after_registry,
        "dry-run must not touch the registry"
    );
    assert_eq!(
        before_ability, after_ability,
        "dry-run must not touch manifests"
    );
}

// ── summarize_schema helper ──────────────────────────────────────────

#[test]
fn summarize_schema_emits_object_keys_with_required_marker() {
    // The one-line shape summary is what the dry-run table
    // shows; the test pins the format so a reader of the
    // summary can tell "required" from "optional" at a glance.
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "prompt": {"type": "string"},
            "context": {"type": "string"}
        },
        "required": ["prompt"]
    });
    assert_eq!(summarize_schema(&schema), "object(context,prompt!)");
}

#[test]
fn summarize_schema_handles_non_object_type() {
    let schema = serde_json::json!({"type": "string"});
    assert_eq!(summarize_schema(&schema), "string");
}

#[test]
fn summarize_schema_handles_object_with_no_properties() {
    let schema = serde_json::json!({"type": "object"});
    assert_eq!(summarize_schema(&schema), "object");
}

#[test]
fn run_add_refuses_when_root_carries_agent_toml_but_registry_empty() {
    // Defensive: someone has `agent.toml` at
    // `<agents_root>/alice/` (maybe copied from another
    // machine, maybe a prior install) but the registry
    // doesn't know about it. We must not silently adopt
    // it — the operator should import it explicitly so
    // they see what runtime / model / description
    // travelled with the file.
    let _g = joined_home();
    // Materialize the directory by hand.
    let root = config::agents_root().join("alice");
    let spec = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
    AgentDirectory::create(&Location::Local { root: root.clone() }, spec).unwrap();
    assert!(root.join("agent.toml").exists());

    let err = run_add(add_args("alice", "claude-code", None))
        .expect_err("must refuse to adopt pre-existing agent.toml");
    let msg = format!("{err}");
    assert!(
        msg.contains("agent.toml") || msg.contains("already"),
        "error must name the conflict; got {msg}"
    );
}

// ── mcp add helpers ────────────────────────────────────────────────

#[test]
fn generated_mcp_ability_name_is_slug_safe_and_deterministic() {
    // Prefix + server + tool slugify independently and join with
    // the dotted-verb convention (single `_` between prefix and
    // server, double `__` between server and tool — operators
    // grep the double underscore to identify the tool half).
    // Note: `slug_segment` collapses `-` to `_` along with other
    // non-alnum punctuation, so `geocode-address` lands as
    // `geocode_address` rather than retaining the hyphen.
    let name = generated_mcp_ability_name("mcp", "Google Maps", "geocode-address");
    assert_eq!(name, "mcp_google_maps_geocode_address");
}

#[test]
fn generated_mcp_ability_name_collapses_runs_of_punctuation() {
    // Internal slug runs collapse to a single separator so the
    // emitted ability name remains a legal verb (no `__` runs
    // sneaking in from messy upstream names).
    let name = generated_mcp_ability_name("MCP", "google//maps", "geo  code");
    assert_eq!(name, "mcp_google_maps_geo_code");
}

#[test]
fn generated_mcp_ability_name_falls_back_to_hash_when_slug_empty() {
    // Upstream pair that slugifies to nothing (e.g. all
    // non-alphanumeric) must still produce a stable, unique
    // ability name so collisions surface as different bindings.
    let a = generated_mcp_ability_name("", "...", "///");
    let b = generated_mcp_ability_name("", "***", "===");
    assert!(a.starts_with("mcp_"), "fallback prefix: {a}");
    assert!(b.starts_with("mcp_"), "fallback prefix: {b}");
    assert_ne!(a, b, "distinct upstream pairs must hash to distinct names");
    // Determinism: same input → same output.
    let a2 = generated_mcp_ability_name("", "...", "///");
    assert_eq!(a, a2);
}

#[test]
fn generated_mcp_ability_name_empty_prefix_drops_leading_separator() {
    // `--prefix=""` should produce `<server>_<tool>` without a
    // leading underscore — operators use the empty prefix when
    // they manage their own naming scheme.
    let name = generated_mcp_ability_name("", "echo", "ping");
    assert_eq!(name, "echo_ping");
}

#[test]
fn existing_mcp_binding_extracts_server_and_tool() {
    // A round-tripped manifest must let an idempotent re-run
    // recognise the prior binding so we skip rewriting it.
    let plan = McpAbilityPlan {
        server: "echo".into(),
        tool: "ping".into(),
        verb: "mcp_echo__ping".into(),
        description: "Echo ping.".into(),
        input_schema: serde_json::json!({"type": "object"}),
        cost: None,
    };
    let body = mcp_manifest_for(&plan).unwrap().to_toml_string().unwrap();
    let binding = existing_mcp_binding(&body).expect("manifest declares an mcp binding");
    assert_eq!(binding, ("echo".to_string(), "ping".to_string()));
}

#[test]
fn existing_mcp_binding_returns_none_for_non_mcp_exec() {
    // A manifest without an `mcp` exec block is the operator's
    // own file — must NOT be treated as "matching binding" and
    // overwritten by the idempotent skip path.
    let manifest_toml = r#"
schema_version = "1"
name = "ping"
description = "Operator-authored manifest."

[input_schema]
type = "object"
"#;
    assert_eq!(existing_mcp_binding(manifest_toml), None);
}

#[test]
fn existing_mcp_binding_returns_none_for_malformed_toml() {
    // Don't panic on garbage on disk; the caller will then fall
    // through to the "refuse to overwrite without --overwrite"
    // branch, which is the safer disposition.
    assert_eq!(existing_mcp_binding("this is not valid toml @@@"), None);
}

#[test]
fn mcp_manifest_for_emits_mcp_exec_with_pinned_server_tool() {
    use crate::daemon::ability::manifest::{AbilityExec, AbilityManifest};
    let plan = McpAbilityPlan {
        server: "echo".into(),
        tool: "ping".into(),
        verb: "mcp_echo__ping".into(),
        description: "Echo ping.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {"text": {"type": "string"}}
        }),
        cost: None,
    };
    let manifest = mcp_manifest_for(&plan).unwrap();
    assert_eq!(manifest.name(), "mcp_echo__ping");
    assert_eq!(manifest.description(), "Echo ping.");
    match manifest.exec() {
        Some(AbilityExec::Mcp(exec)) => {
            assert_eq!(exec.server, "echo");
            assert_eq!(exec.tool, "ping");
        }
        other => panic!("expected Mcp exec, got {other:?}"),
    }
    // Round-trip through TOML must preserve the binding so
    // existing_mcp_binding can read it back.
    let body = manifest.to_toml_string().unwrap();
    let reparsed = AbilityManifest::from_toml_str(&body).unwrap();
    assert_eq!(reparsed.name(), "mcp_echo__ping");
}

#[test]
fn assert_tools_filter_satisfied_passes_when_every_request_resolved() {
    let planned = vec![
        McpAbilityPlan {
            server: "s".into(),
            tool: "a".into(),
            verb: "_".into(),
            description: String::new(),
            input_schema: serde_json::json!({}),
            cost: None,
        },
        McpAbilityPlan {
            server: "s".into(),
            tool: "b".into(),
            verb: "_".into(),
            description: String::new(),
            input_schema: serde_json::json!({}),
            cost: None,
        },
    ];
    assert_tools_filter_satisfied(&["a".into(), "b".into()], &planned).unwrap();
}

#[test]
fn assert_tools_filter_satisfied_empty_filter_is_unconditional_ok() {
    assert_tools_filter_satisfied(&[], &[]).unwrap();
}

#[test]
fn assert_tools_filter_satisfied_lists_every_missing_tool() {
    let planned = vec![McpAbilityPlan {
        server: "s".into(),
        tool: "a".into(),
        verb: "_".into(),
        description: String::new(),
        input_schema: serde_json::json!({}),
        cost: None,
    }];
    let err = assert_tools_filter_satisfied(
        &["a".into(), "missing-1".into(), "missing-2".into()],
        &planned,
    )
    .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("missing-1"),
        "msg should name missing-1: {msg}"
    );
    assert!(
        msg.contains("missing-2"),
        "msg should name missing-2: {msg}"
    );
    assert!(
        !msg.contains(" a,") && !msg.ends_with(" a"),
        "msg should not list resolved tools as missing: {msg}"
    );
}

// ── cost flags ──────────────────────────────────────────────────────

#[test]
fn cost_kind_arg_round_trips_to_core_cost_kind() {
    // Pins the CLI ↔ core enum lockstep documented on
    // `CostKindArg`. If a future variant lands on
    // `daemon::ability::manifest::CostKind` and someone forgets the
    // mirror, this test fails loud instead of leaving operators
    // with an unreachable flag.
    use crate::daemon::ability::manifest::CostKind;
    assert_eq!(CostKindArg::Free.into_core(), CostKind::Free);
    assert_eq!(
        CostKindArg::ExternalMetered.into_core(),
        CostKind::ExternalMetered
    );
    assert_eq!(CostKindArg::LlmMetered.into_core(), CostKind::LlmMetered);
    assert_eq!(CostKindArg::Unknown.into_core(), CostKind::Unknown);
}

#[test]
fn build_cost_meta_returns_none_when_kind_absent() {
    // No `--cost-kind` → no `[cost]` table on disk. A bare
    // `--cost-label` without a kind is rejected at clap parse time
    // by `requires = "cost_kind"`, so the helper does not need to
    // re-defend against that case; we just confirm the None path.
    assert!(build_cost_meta(None, None).unwrap().is_none());
    assert!(build_cost_meta(None, Some("ignored")).unwrap().is_none());
}

#[test]
fn build_cost_meta_normalises_blank_label_to_none() {
    // CLI users can technically pass `--cost-label ""` or just
    // whitespace; the manifest validator would reject an empty
    // label outright. Treat empty-ish input as "label omitted" so
    // the kind still lands without dragging a useless blank
    // string onto disk.
    use crate::daemon::ability::manifest::CostKind;
    let meta = build_cost_meta(Some(CostKindArg::ExternalMetered), Some("   "))
        .unwrap()
        .expect("kind set => meta present");
    assert_eq!(meta.kind, CostKind::ExternalMetered);
    assert!(meta.label.is_none());
}

#[test]
fn build_cost_meta_carries_kind_and_trimmed_label() {
    use crate::daemon::ability::manifest::CostKind;
    let meta = build_cost_meta(
        Some(CostKindArg::ExternalMetered),
        Some("  Google Maps API — $5 per 1000 requests  "),
    )
    .unwrap()
    .expect("kind set => meta present");
    assert_eq!(meta.kind, CostKind::ExternalMetered);
    // We keep the inner spacing verbatim; only outer whitespace
    // is normalised so a label written with deliberate alignment
    // (rare, but plausible) survives.
    assert_eq!(
        meta.label.as_deref(),
        Some("Google Maps API — $5 per 1000 requests")
    );
}

#[test]
fn mcp_manifest_for_stamps_declared_cost_on_disk() {
    // Operator passed `--cost-kind external-metered --cost-label
    // "Google Maps geocoding — $5/1000"`. The generated TOML must
    // carry that `[cost]` table verbatim, and re-parsing it must
    // surface the same `CostMeta` — that is what `profiles::mcp`
    // reads to stop reporting `cost: unknown` on this row.
    use crate::daemon::ability::manifest::{AbilityManifest, CostKind, CostMeta};
    let plan = McpAbilityPlan {
        server: "Google Maps MCP".into(),
        tool: "geocode-address".into(),
        verb: "mcp_google_maps_mcp__geocode_address".into(),
        description: "Geocode an address via Google Maps.".into(),
        input_schema: serde_json::json!({"type": "object"}),
        cost: Some(CostMeta {
            kind: CostKind::ExternalMetered,
            label: Some("Google Maps geocoding — $5/1000 requests".into()),
        }),
    };
    let manifest = mcp_manifest_for(&plan).unwrap();
    let cost = manifest
        .cost()
        .expect("declared cost must survive manifest build");
    assert_eq!(cost.kind, CostKind::ExternalMetered);
    assert_eq!(
        cost.label.as_deref(),
        Some("Google Maps geocoding — $5/1000 requests")
    );
    // Round-trip through TOML — `agent mcp add` writes via
    // `to_toml_string`; reading happens via `from_toml_str` at
    // the next daemon boot. Any drift between the two surfaces
    // here as a deserialise failure or label mismatch.
    let body = manifest.to_toml_string().unwrap();
    assert!(
        body.contains("[cost]") && body.contains("external_metered"),
        "manifest TOML must contain a [cost] table with kind = external_metered, got:\n{body}"
    );
    let reparsed = AbilityManifest::from_toml_str(&body).unwrap();
    let reparsed_cost = reparsed.cost().expect("cost survives round-trip");
    assert_eq!(reparsed_cost.kind, CostKind::ExternalMetered);
    assert_eq!(
        reparsed_cost.label.as_deref(),
        Some("Google Maps geocoding — $5/1000 requests")
    );
}

#[test]
fn mcp_manifest_for_without_cost_writes_no_cost_table() {
    // Default — no `--cost-kind` — keeps the on-disk manifest free
    // of any `[cost]` section so the runtime applies its
    // honesty-rule inference (`unknown` for MCP-backed tools).
    // We pin this to prevent a future regression where someone
    // "helpfully" stamps a default cost into every generated file.
    let plan = McpAbilityPlan {
        server: "echo".into(),
        tool: "ping".into(),
        verb: "mcp_echo__ping".into(),
        description: "Echo ping.".into(),
        input_schema: serde_json::json!({"type": "object"}),
        cost: None,
    };
    let body = mcp_manifest_for(&plan).unwrap().to_toml_string().unwrap();
    assert!(
        !body.contains("[cost]"),
        "expected no [cost] table when --cost-kind omitted; got:\n{body}"
    );
}
