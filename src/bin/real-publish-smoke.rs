// real-publish-smoke — exercise the per-user-agent advertise path
// against the user's actual on-disk registry, capturing every
// federation.advertise_* call that would have been emitted.
//
// What this binary proves
// -----------------------
// The unit tests in publish.rs use a synthetic AgentRegistry. This
// binary instead reads `~/.easynet/agents.json` and `credentials.json`
// straight off disk, builds the same BootstrapPlan the daemon uses,
// runs `republish_abilities_via_advertise`, and routes every call
// into a recording invoker rather than the real Axon bridge.
//
// Pass criteria for the user-agent ability fix:
//   * One `federation.advertise_abilities@1` call per user agent
//   * Owner URI in that payload matches the user-agent URA bootstrap
//     minted in local-agents.json
//   * The abilities array contains `<agent>.chat`
//
// We can't talk to a hub from here — that needs a live federation —
// but we can prove the daemon's exact call sequence against the
// operator's exact installed agents. The transport is the only
// thing diverted; everything upstream is the production code path.

use std::cell::RefCell;
use std::path::PathBuf;

use serde_json::Value;

use easynet_cli::runtime::advertise::AbilityInvoker;
use easynet_cli::runtime::agents::profiles::bootstrap::{BootstrapPlan, LlmSubAgent};
use easynet_cli::runtime::publish::republish_abilities_via_advertise;

struct RecordingInvoker {
    calls: RefCell<Vec<(String, Value)>>,
}

impl RecordingInvoker {
    fn new() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
        }
    }
    fn into_calls(self) -> Vec<(String, Value)> {
        self.calls.into_inner()
    }
}

impl AbilityInvoker for RecordingInvoker {
    fn invoke_ability(
        &self,
        _tenant_id: &str,
        resource_uri: &str,
        payload_json: Value,
    ) -> Result<Value, String> {
        self.calls
            .borrow_mut()
            .push((resource_uri.to_string(), payload_json));
        Ok(serde_json::json!({"ack": true, "replaced_prior": false}))
    }
}

fn easynet_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join(".easynet"))
        .expect("HOME must be set")
}

fn read_json(path: &std::path::Path) -> anyhow::Result<Value> {
    let s = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    serde_json::from_str(&s).map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))
}

fn main() -> anyhow::Result<()> {
    let dir = easynet_dir();

    // Load credentials.
    let creds = read_json(&dir.join("credentials.json"))?;
    let tenant_id = creds["tenant_id"].as_str().unwrap_or("default").to_string();
    let node_id = creds["node_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("credentials.json missing node_id"))?
        .to_string();

    println!("== real-publish-smoke ==");
    println!("tenant_id        = {tenant_id}");
    println!("node_id          = {node_id}");

    // Load registered agents.
    let agents_json = read_json(&dir.join("agents.json"))?;
    let agents_obj = agents_json["agents"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("agents.json missing `agents` object"))?;
    let llm_sub_agents: Vec<LlmSubAgent> = agents_obj
        .iter()
        .map(|(name, entry)| LlmSubAgent {
            name: name.clone(),
            agent_type_display: entry["agent_type"]
                .as_str()
                .unwrap_or("claude-code")
                .to_string(),
        })
        .collect();
    let agent_names: Vec<&str> = llm_sub_agents.iter().map(|s| s.name.as_str()).collect();
    println!("realm            = {tenant_id}");
    println!("host_device_uri  = {node_id}");
    println!("llm_sub_agents   = {agent_names:?}");
    println!();

    let plan = BootstrapPlan {
        realm: tenant_id.clone(),
        // Smoke test: synthetic user id; real boot reads from
        // creds.username (carries the user-uuid in v4.1.4).
        user_id: "smoke-user".to_string(),
        host_device_uri: node_id.clone(),
        consent: true,
        policy: false,
        mcp: false,
        llm_sub_agents,
    };

    let invoker = RecordingInvoker::new();
    let outcomes = republish_abilities_via_advertise(&invoker, &tenant_id, &plan);

    let calls = invoker.into_calls();
    println!(
        "== {} federation.advertise_* calls captured ==",
        calls.len()
    );
    println!();

    println!("-- advertise_agent --");
    for (uri, payload) in &calls {
        if uri.contains("federation.advertise_agent@1") {
            let agent_uri = payload["agent_uri"].as_str().unwrap_or("?");
            let auth = &payload["signing_authority"];
            println!("  agent_uri = {agent_uri}");
            println!("    signing_authority = {auth}");
        }
    }
    println!();

    println!("-- advertise_abilities --");
    for (uri, payload) in &calls {
        if uri.contains("federation.advertise_abilities@1") {
            let owner = payload["agent_uri"].as_str().unwrap_or("?");
            let abilities = payload["abilities"].as_array().cloned().unwrap_or_default();
            let names: Vec<&str> = abilities
                .iter()
                .filter_map(|a| a["name"].as_str())
                .collect();
            println!("  owner = {owner}  ({} abilities)", names.len());
            for n in &names {
                println!("    - {n}");
            }
        }
    }
    println!();

    // Verification — re-read local-agents.json (republish updated it).
    let local_agents = read_json(&dir.join("local-agents.json"))?;
    let hosted = local_agents["hosted_agents"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    println!("-- verification --");
    let mut all_pass = true;
    for sub in &plan.llm_sub_agents {
        let agent_name = &sub.name;
        let expected_chat = format!("{agent_name}.chat");
        let agent_uri = hosted
            .iter()
            .find(|e| {
                e["profile"].as_str() == Some("llm")
                    && e["name"].as_str() == Some(agent_name.as_str())
            })
            .and_then(|e| e["agent_uri"].as_str().map(|s| s.to_string()));

        let agent_uri = match agent_uri {
            Some(u) => u,
            None => {
                println!("  ❌ {agent_name}: no URA in local-agents.json (bootstrap mint failed?)");
                all_pass = false;
                continue;
            }
        };

        let found = calls.iter().any(|(uri, payload)| {
            uri.contains("federation.advertise_abilities@1")
                && payload["agent_uri"].as_str() == Some(agent_uri.as_str())
                && payload["abilities"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .any(|a| a["name"].as_str() == Some(expected_chat.as_str()))
                    })
                    .unwrap_or(false)
        });

        if found {
            println!(
                "  ✓ {agent_name}: advertise_abilities owner={agent_uri} contains {expected_chat}"
            );
        } else {
            println!("  ❌ {agent_name}: did NOT find {expected_chat} under owner={agent_uri}");
            all_pass = false;
        }
    }
    println!();

    let total = outcomes.len();
    let ok = outcomes.iter().filter(|o| o.result.is_ok()).count();
    let err = total - ok;
    println!("PublishOutcome rows: {total} total, {ok} Ok, {err} Err");
    for o in &outcomes {
        if let Err(msg) = &o.result {
            if o.label == "local-agents.json" {
                continue;
            }
            println!("    ! {label}: {msg}", label = o.label);
        }
    }
    println!();

    if all_pass {
        println!(
            "RESULT: PASS — every user agent has its <agent>.chat advertised under its own URA"
        );
        Ok(())
    } else {
        anyhow::bail!("RESULT: FAIL — at least one user agent's chat ability is not advertised")
    }
}
