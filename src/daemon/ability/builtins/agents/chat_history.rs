// EasyNet CLI — chat.history.{list,get} handlers
// =================================================================
//
// File: src/daemon/ability/builtins/agents/chat_history.rs
// Description: Two device-level read-only abilities that expose the
//              per-agent chat transcripts already persisted by the
//              chat ability to `~/.easynet/agents/<agent>/sessions/`.
//
//   * `chat.history.list` (RPC) — list an agent's chat sessions
//                                  (id, started_at, last_turn_at,
//                                  turn_count, prompt_preview),
//                                  most-recent-first.
//   * `chat.history.get`  (RPC) — read one session's JSONL turns
//                                  (the session_meta line + every
//                                  turn record), verbatim.
//
// The persistence + readers already exist in
// `crate::persistence::chat_sessions`; this module only registers
// them as invokable abilities so the Hub/backend (and ultimately the
// Frontend Group page) can read transcripts over the wire instead of
// only via the local `easynet agent chat-history` CLI command.
//
// Owner is Device: transcripts are device-local files, read off
// whichever device hosted the agent — matching `session.list`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::{AxonAbilityCatalog, OwnerKind};

pub const ABILITY_LIST: &str = crate::daemon::ability::names::agents::CHAT_HISTORY_LIST;
pub const ABILITY_GET: &str = crate::daemon::ability::names::agents::CHAT_HISTORY_GET;

/// Register the two chat-history read abilities. Called from
/// `daemon::ability::catalog::build_registry`.
pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner(
        ABILITY_LIST,
        OwnerKind::Device,
        std::sync::Arc::new(list_handler),
    );
    reg.register_rpc_with_owner(
        ABILITY_GET,
        OwnerKind::Device,
        std::sync::Arc::new(get_handler),
    );
}

/// `chat.history.list` — args `{ "agent": string }`.
/// Returns `{ "agent": string, "lifelong_session_id": string|null,
/// "sessions": [SessionDescriptor, ...] }`. `lifelong_session_id`
/// names the session bound as the agent's lifelong default thread
/// (null until the first lifelong turn), so the Frontend can open it
/// by default and badge it in the session list.
fn list_handler(args: Value) -> anyhow::Result<Value> {
    let agent = require_agent(&args)?;
    let sessions = crate::persistence::chat_sessions::list_sessions(&agent);
    let json_sessions: Vec<Value> = sessions
        .iter()
        .map(|s| serde_json::to_value(s).unwrap_or(Value::Null))
        .collect();
    let lifelong = crate::persistence::chat_sessions::lifelong_session(&agent);
    Ok(json!({
        "agent": agent,
        "lifelong_session_id": lifelong,
        "sessions": json_sessions,
    }))
}

/// `chat.history.get` — args `{ "agent": string, "session_id": string }`.
/// Returns `{ "agent", "session_id", "turns": [<jsonl value>, ...] }`
/// where each value is one verbatim JSONL line (the leading
/// `session_meta` row plus every `turn` record).
fn get_handler(args: Value) -> anyhow::Result<Value> {
    let agent = require_agent(&args)?;
    let session_id = args
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("chat.history.get: `session_id` required"))?
        .to_string();
    let turns = crate::persistence::chat_sessions::load_session(&agent, &session_id)?;
    Ok(json!({ "agent": agent, "session_id": session_id, "turns": turns }))
}

fn require_agent(args: &Value) -> anyhow::Result<String> {
    args.get("agent")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(normalize_agent)
        .ok_or_else(|| anyhow::anyhow!("chat.history: `agent` required"))
}

/// Accept either the local agent name (`caesura`) or the canonical
/// Agent URA (`easynet:///r/<realm>/agent/<user>.<agent>`) and project
/// both to the local registry name that keys the on-disk session store
/// (`~/.easynet/agents/<name>/sessions/`). The Frontend sends the URA —
/// its `agent_id` field is canonical per RFC-001 — while the CLI sends
/// the bare name; without this projection the URA was used as a
/// directory name and every UI history read came back empty. Parsing
/// stays with Axon (`parse_ura`); non-URA strings pass through as-is.
fn normalize_agent(raw: &str) -> String {
    if let Ok(parsed) = crate::ura::parse_ura(raw) {
        if parsed.kind == crate::ura::URAKind::Agent {
            if let Some((_user_id, agent_id)) = parsed.agent_ids() {
                return agent_id.to_string();
            }
        }
    }
    raw.to_string()
}

pub fn list_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "agent": {"type": "string", "description": "Agent whose chat sessions to list — local name (`caesura`) or canonical Agent URA."}
        },
        "required": ["agent"],
        "additionalProperties": false,
    })
}

pub fn get_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "agent": {"type": "string", "description": "Agent that owns the session — local name or canonical Agent URA."},
            "session_id": {"type": "string", "description": "Session id to read (from chat.history.list)."}
        },
        "required": ["agent", "session_id"],
        "additionalProperties": false,
    })
}

pub fn list_description() -> &'static str {
    "List an agent's persisted chat sessions (id, started_at, last_turn_at, turn_count, \
     prompt_preview), most-recent-first."
}

pub fn get_description() -> &'static str {
    "Read one chat session's transcript turns (the session_meta line plus every recorded \
     turn: prompt, reply, tool_calls, usage), verbatim."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_requires_agent() {
        let err = list_handler(json!({})).unwrap_err();
        assert!(err.to_string().contains("agent"));
    }

    #[test]
    fn list_and_get_accept_canonical_agent_ura() {
        // The Frontend's `agent_id` is the canonical Agent URA; the
        // on-disk session store is keyed by the local name. Both
        // handlers must project URA → local name, or every UI history
        // read silently comes back empty (the regression that hid the
        // lifelong thread after a reload).
        let _g = crate::cli::test_support::HomeGuard::new();
        crate::persistence::chat_sessions::write_turn(
            "demot",
            "sess-1",
            "hi",
            "yo",
            &[],
            &json!({}),
        )
        .expect("seed turn");
        crate::persistence::chat_sessions::set_lifelong_session("demot", "sess-1").expect("bind");
        let ura = crate::ura::agent_ura("localhost", "dev", "demot");
        let resp = list_handler(json!({"agent": ura.as_str()})).expect("list via URA");
        assert_eq!(resp["agent"], "demot");
        assert_eq!(resp["lifelong_session_id"], "sess-1");
        assert_eq!(resp["sessions"].as_array().map(Vec::len), Some(1));
        let resp = get_handler(json!({"agent": ura.as_str(), "session_id": "sess-1"}))
            .expect("get via URA");
        assert_eq!(
            resp["turns"].as_array().map(Vec::len),
            Some(2),
            "meta + 1 turn"
        );
    }

    #[test]
    fn list_surfaces_lifelong_session_id() {
        let _g = crate::cli::test_support::HomeGuard::new();
        // Unbound: explicit null, not a missing key — the Frontend
        // reads the field unconditionally.
        let resp = list_handler(json!({"agent": "demot"})).expect("list");
        assert!(resp["lifelong_session_id"].is_null());
        crate::persistence::chat_sessions::set_lifelong_session("demot", "sess-1").expect("bind");
        let resp = list_handler(json!({"agent": "demot"})).expect("list");
        assert_eq!(resp["lifelong_session_id"], "sess-1");
    }

    #[test]
    fn get_requires_agent_and_session_id() {
        assert!(get_handler(json!({"session_id": "s1"}))
            .unwrap_err()
            .to_string()
            .contains("agent"));
        assert!(get_handler(json!({"agent": "demo"}))
            .unwrap_err()
            .to_string()
            .contains("session_id"));
    }

    #[test]
    fn schemas_declare_required_args() {
        assert_eq!(list_input_schema()["required"][0], "agent");
        let get_schema = get_input_schema();
        let get_req = get_schema["required"].as_array().unwrap();
        assert!(get_req.iter().any(|v| v == "agent"));
        assert!(get_req.iter().any(|v| v == "session_id"));
    }
}
