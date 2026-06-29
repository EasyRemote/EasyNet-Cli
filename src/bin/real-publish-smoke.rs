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
//   * One `federation.advertise_abilities` call per user agent
//   * Owner URA in that payload matches the user-agent URA bootstrap
//     minted in local-agents.json
//   * The ability_summaries array contains `chat`
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
        resource_ura: &str,
        payload_json: Value,
    ) -> Result<Value, String> {
        self.calls
            .borrow_mut()
            .push((resource_ura.to_string(), payload_json));
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

fn payload_owner_ura(payload: &Value) -> Option<&str> {
    payload["owner_ura"]
        .as_str()
        .or_else(|| payload["agent_ura"].as_str())
}

fn ability_summary_public_name(value: &Value) -> Option<String> {
    let local_name = value["local_name"].as_str()?.trim();
    if local_name.is_empty() {
        return None;
    }
    let namespace = value["namespace"].as_str().unwrap_or("").trim();
    if namespace.is_empty() {
        Some(local_name.to_string())
    } else {
        Some(format!("{namespace}.{local_name}"))
    }
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
            model: entry["model"].as_str().map(ToString::to_string),
        })
        .collect();
    let agent_names: Vec<&str> = llm_sub_agents.iter().map(|s| s.name.as_str()).collect();
    println!("realm            = {tenant_id}");
    println!("host_device_ura  = {node_id}");
    println!("llm_sub_agents   = {agent_names:?}");
    println!();

    let plan = BootstrapPlan {
        realm: tenant_id.clone(),
        // Smoke test: synthetic user id; real boot reads from
        // credentials and keeps the UUID subject separate from the
        // username owner-prefix.
        user_id: "smoke-user".to_string(),
        username: "smoke".to_string(),
        host_device_ura: node_id.clone(),
        consent: true,
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
    for (ura, payload) in &calls {
        if ura.contains("federation.advertise_agent") {
            let agent_ura = payload["agent_ura"].as_str().unwrap_or("?");
            let auth = &payload["signing_authority"];
            println!("  agent_ura = {agent_ura}");
            println!("    signing_authority = {auth}");
        }
    }
    println!();

    println!("-- advertise_abilities --");
    for (ura, payload) in &calls {
        if ura.contains("federation.advertise_abilities") {
            let owner = payload_owner_ura(payload).unwrap_or("?");
            let ability_summaries = payload["ability_summaries"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let names: Vec<String> = ability_summaries
                .iter()
                .filter_map(ability_summary_public_name)
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
        let expected_chat = "chat";
        let agent_ura = hosted
            .iter()
            .find(|e| {
                e["profile"].as_str() == Some("llm")
                    && e["name"].as_str() == Some(agent_name.as_str())
            })
            .and_then(|e| e["agent_ura"].as_str().map(|s| s.to_string()));

        let agent_ura = match agent_ura {
            Some(u) => u,
            None => {
                println!("  ❌ {agent_name}: no URA in local-agents.json (bootstrap mint failed?)");
                all_pass = false;
                continue;
            }
        };

        let found = calls.iter().any(|(ura, payload)| {
            ura.contains("federation.advertise_abilities")
                && payload_owner_ura(payload) == Some(agent_ura.as_str())
                && payload["ability_summaries"]
                    .as_array()
                    .map(|arr| {
                        arr.iter().any(|a| {
                            ability_summary_public_name(a).as_deref() == Some(expected_chat)
                        })
                    })
                    .unwrap_or(false)
        });

        if found {
            println!(
                "  ✓ {agent_name}: advertise_abilities owner={agent_ura} contains {expected_chat}"
            );
        } else {
            println!("  ❌ {agent_name}: did NOT find {expected_chat} under owner={agent_ura}");
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
