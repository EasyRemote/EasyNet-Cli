// EasyNet CLI — mission.discuss_round orchestration ability
// ===========================================================
//
// File: src/runtime/system_abilities/automation/orchestration.rs
// Description: One ability — `mission.discuss_round` — that runs
//              one human-bracketed sub-turn of a multi-agent
//              discussion. The CLI surface (`easynet mission
//              discuss …`) is a thin shell that creates a room
//              via `discuss.create`, posts the human's turn via
//              `discuss.post`, then invokes this ability to let
//              the agents speak among themselves until they all
//              skip or `max_cycles` is reached.
//
// Why this is a single ability and not a CLI-side loop
// -----------------------------------------------------
// Per the ability-only ontology, every coherent unit of work the
// system performs is one ability invocation. A "sub-turn" — the
// closed period between two human turns where N agents talk among
// themselves — is exactly such a unit: it has well-defined inputs
// (room_id, agents, max_cycles), well-defined termination (all-skip
// or budget exhausted), and a single structured output (the cycle
// log + errors envelope). Lifting it to an ability means EAL,
// MCP, and CLI all drive discussions through the same surface.
//
// Concurrency model: cycle-start snapshot, parallel agent queries
// ----------------------------------------------------------------
// Within one cycle every agent is queried in parallel against the
// SAME transcript snapshot taken at cycle start. This is the
// load-bearing fairness property: an agent's prompt for cycle N
// does NOT see other agents' cycle-N replies (those land in the
// transcript while this cycle's prompts were already frozen).
// Cycle N+1 reads a fresh snapshot which DOES include cycle-N
// replies, so the discussion progresses without becoming a
// "whoever-replies-first wins" race.
//
// Live visibility through the room is preserved separately: as
// each agent completes its cycle-N reply, we immediately
// `discuss.post` it. A human watching `discuss.subscribe` sees
// the replies stream in real time, even though same-cycle peers
// did not see them as input.
//
// Skip protocol
// -------------
// An agent's reply is treated as skip when:
//   * `reply.trim().eq_ignore_ascii_case("[SKIP]")`, OR
//   * `reply` is empty (defence-in-depth — LLMs rarely emit a
//     truly empty reply, but if they do, treating it as skip is
//     the only sensible interpretation).
// Anything else is treated as the agent's spoken contribution
// for this cycle. Prompt teaches the convention literally:
// "reply with `[SKIP]` alone if you have nothing to add."
//
// Failure handling
// ----------------
// One agent's chat failure (transient timeout, driver crash, …)
// is recorded in the response's `errors` array and treated as
// skip for that cycle. Subsequent cycles still query that agent
// — the failure may have been transient. Sub-turn termination
// remains "everyone skipped this cycle" (which may now mean
// "everyone skipped or failed").
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::AxonAbilityCatalog;
use crate::runtime::ability_dispatch::OwnerKind;
use crate::runtime::execution::discuss::DiscussService;

pub const ABILITY_DISCUSS_ROUND: &str =
    crate::daemon::ability::names::automation::MISSION_DISCUSS_ROUND;

/// Default upper bound on cycles per sub-turn. Generous enough that
/// a healthy discussion converges (3–5 cycles typical), small
/// enough that a runaway loop terminates quickly. Caller can
/// override via the `max_cycles` arg.
const DEFAULT_MAX_CYCLES: u32 = 10;

/// Hard cap on `max_cycles` regardless of caller input. A typo
/// like `max_cycles: 9999999999` cannot allocate a corresponding
/// number of agent chat calls; this is the safety governor.
const HARD_MAX_CYCLES: u32 = 100;

type DispatchRegistryHandle = Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>;

/// Owns `mission.discuss_round` runtime state for one ability registry.
///
/// Invariant 1: `agent_sessions` is keyed by `(room_id, agent_name)` and
/// stores only the chat session id returned by that agent's `<agent>.chat`
/// ability.
/// Invariant 2: session continuity is scoped to this service instance, so
/// separate registry builds do not share hidden orchestration state.
/// Invariant 3: nested chat calls dispatch through the registry handle
/// populated by the daemon build site; this service never opens a recursive
/// daemon IPC connection.
#[derive(Debug)]
struct OrchestrationService {
    discuss: Arc<DiscussService>,
    dispatch_registry_handle: DispatchRegistryHandle,
    agent_sessions: Mutex<HashMap<(String, String), String>>,
}

impl OrchestrationService {
    fn new(discuss: Arc<DiscussService>, dispatch_registry_handle: DispatchRegistryHandle) -> Self {
        Self {
            discuss,
            dispatch_registry_handle,
            agent_sessions: Mutex::new(HashMap::new()),
        }
    }

    fn prior_session(&self, room_id: &str, agent: &str) -> Option<String> {
        self.agent_sessions
            .lock()
            .ok()
            .and_then(|m| m.get(&(room_id.to_string(), agent.to_string())).cloned())
    }

    fn remember_session(&self, room_id: &str, agent: &str, session_id: String) {
        if let Ok(mut sessions) = self.agent_sessions.lock() {
            sessions.insert((room_id.to_string(), agent.to_string()), session_id);
        }
    }
}

/// Register `mission.discuss_round`. Called once at daemon boot
/// from `daemon::ability::catalog::build_registry_with_services`. The
/// `dispatch_registry_handle` is populated AFTER `Arc::new(reg)`
/// so the handler can dispatch into peer abilities in-process —
/// going back through the IPC client would deadlock the
/// daemon's single-thread accept loop while waiting on a nested
/// chat call.
pub fn register(
    reg: &mut AxonAbilityCatalog,
    discuss: Arc<DiscussService>,
    dispatch_registry_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
) {
    let service = Arc::new(OrchestrationService::new(discuss, dispatch_registry_handle));
    reg.register_rpc_with_owner(
        "mission.discuss_round",
        OwnerKind::Device,
        Arc::new(move |args| service.discuss_round(args)),
    );
}

// ── Service methods ─────────────────────────────────────────────

impl OrchestrationService {
    fn discuss_round(self: &Arc<Self>, args: Value) -> anyhow::Result<Value> {
        let room_id = args
            .get("room_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("mission.discuss_round: `room_id` is required"))?
            .to_string();
        let agents: Vec<String> = args
            .get("agents")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                anyhow::anyhow!("mission.discuss_round: `agents` (array of strings) is required")
            })?
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect();
        if agents.is_empty() {
            anyhow::bail!("mission.discuss_round: `agents` must contain at least one name");
        }
        let max_cycles = args
            .get("max_cycles")
            .and_then(Value::as_u64)
            .map(|n| n.min(HARD_MAX_CYCLES as u64) as u32)
            .unwrap_or(DEFAULT_MAX_CYCLES);
        if max_cycles == 0 {
            anyhow::bail!("mission.discuss_round: `max_cycles` must be ≥ 1");
        }

        let topic = args
            .get("topic")
            .and_then(Value::as_str)
            .map(str::to_string);

        // Optional caller-supplied role assignments. Shape:
        // `{ "<agent>": "<role description>", ... }`. When an agent
        // appears in this map its first-cycle prompt skips the
        // self-nomination block and tells it the role is already
        // chosen by the operator. Absent entries trigger the
        // self-nomination prompt path.
        let roles: HashMap<String, String> = args
            .get("roles")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        let mut errors: Vec<Value> = Vec::new();
        let mut speakers_per_cycle: Vec<Vec<String>> = Vec::new();
        let mut termination = "max_cycles_reached";

        for cycle in 1..=max_cycles {
            // Cycle-start snapshot: every agent this cycle sees the
            // same transcript. Replies posted within this cycle do not
            // re-enter same-cycle peers' prompts — they land in the
            // room (visible to discuss.subscribe streamers) and become
            // input on cycle N+1.
            let snapshot = self
                .discuss
                .turns_from(&crate::core::domain::RoomId::new(&room_id), 0)
                .map_err(|e| anyhow::anyhow!("read room transcript: {e}"))?;
            let snapshot_str = render_transcript(&snapshot);

            // Run all agents in parallel for this cycle using OS
            // threads. We deliberately do NOT spin up a nested tokio
            // runtime here: this handler already executes on the
            // daemon's main tokio worker thread, and `Builder::new_*
            // ... build()` panics with "Cannot start a runtime from
            // within a runtime". The chat handler we resolve in
            // `run_agent_cycle` is a sync closure (it returns a
            // `Value`, not a Future); std::thread::spawn is the
            // correct primitive. Failures are caught per-thread and
            // turned into skip + an entry in `errors`.
            let mut handles: Vec<(
                String,
                std::thread::JoinHandle<Result<AgentCycleOutcome, String>>,
            )> = Vec::with_capacity(agents.len());
            for agent in &agents {
                let request = AgentCycleRequest {
                    room_id: room_id.clone(),
                    agent: agent.clone(),
                    agents: agents.clone(),
                    cycle,
                    max_cycles,
                    transcript: snapshot_str.clone(),
                    topic: topic.clone(),
                    assigned_role: roles.get(agent).cloned(),
                };
                let service = Arc::clone(self);
                let join = std::thread::spawn(move || service.run_agent_cycle(request));
                handles.push((agent.clone(), join));
            }

            let cycle_results: Vec<(String, Result<AgentCycleOutcome, String>)> = handles
                .into_iter()
                .map(|(agent, h)| {
                    let r = match h.join() {
                        Ok(inner) => inner,
                        Err(panic_payload) => {
                            // Try to surface the panic message. `JoinHandle::join`
                            // returns Err with a `Box<dyn Any + Send>`; the
                            // common payload types are `&'static str` and
                            // `String`. Falling back to a generic note when
                            // neither matches.
                            let msg = if let Some(s) = panic_payload.downcast_ref::<&'static str>()
                            {
                                (*s).to_string()
                            } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "non-string panic payload".to_string()
                            };
                            Err(format!("agent {agent:?} thread panicked: {msg}"))
                        }
                    };
                    (agent, r)
                })
                .collect();

            let mut spoke_this_cycle: Vec<String> = Vec::new();
            let mut anyone_spoke = false;
            for (agent, result) in cycle_results {
                match result {
                    Ok(AgentCycleOutcome::Speak(text)) => {
                        let post_res = self.discuss.post(
                            &crate::core::domain::RoomId::new(&room_id),
                            crate::core::domain::AgentId::new(&agent),
                            text,
                            None,
                        );
                        match post_res {
                            Ok(_seq) => {
                                spoke_this_cycle.push(agent.clone());
                                anyone_spoke = true;
                            }
                            Err(e) => {
                                errors.push(json!({
                                    "agent":  agent,
                                    "cycle":  cycle,
                                    "phase":  "post",
                                    "error":  format!("{e}"),
                                }));
                            }
                        }
                    }
                    Ok(AgentCycleOutcome::Skip) => {
                        // No-op; reflected in cycle log only as
                        // "this agent did not appear in spoke_this_cycle".
                    }
                    Err(msg) => {
                        errors.push(json!({
                            "agent":  agent,
                            "cycle":  cycle,
                            "phase":  "chat",
                            "error":  msg,
                        }));
                    }
                }
            }
            speakers_per_cycle.push(spoke_this_cycle);

            if !anyone_spoke {
                termination = "all_agents_skipped";
                break;
            }
        }

        let cycles_used = speakers_per_cycle.len() as u32;
        let agents_who_spoke: Vec<String> = {
            let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for c in &speakers_per_cycle {
                for a in c {
                    set.insert(a.clone());
                }
            }
            set.into_iter().collect()
        };

        // Surface the full transcript snapshot in the response so the
        // CLI doesn't have to make a second IPC call to read the turns
        // back — that second call risks landing on a different
        // process's listener if `<agent>.chat` spawned an mcp-serve
        // subprocess that joined the control.sock accept queue while
        // this round was running. Embedding the turns avoids the race
        // entirely; callers that want only structural data can still
        // ignore the field.
        let final_turns = self
            .discuss
            .turns_from(&crate::core::domain::RoomId::new(&room_id), 0)
            .map(|ts| {
                ts.into_iter()
                    .map(|t| serde_json::to_value(t).unwrap_or(Value::Null))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Ok(json!({
            "room_id":           room_id,
            "cycles_used":       cycles_used,
            "max_cycles":        max_cycles,
            "terminated_reason": termination,
            "agents_who_spoke":  agents_who_spoke,
            "speakers_per_cycle": speakers_per_cycle,
            "errors":            errors,
            "turns":             final_turns,
        }))
    }

    /// Run one (agent, cycle) — synthesise the prompt, invoke
    /// `<agent>.chat` over IPC with the per-(room, agent) session_id,
    /// classify the reply.
    ///
    /// Returns `Err(String)` only when the chat invocation itself
    /// failed (transport / driver / agent not registered) — those map
    /// to `errors[]` in the envelope. Skip is `Ok(Skip)`, normal
    /// speech is `Ok(Speak(reply))`.
    fn run_agent_cycle(&self, request: AgentCycleRequest) -> Result<AgentCycleOutcome, String> {
        // Resume per-(room, agent) chat session so the agent's own
        // history (its prior cycles' reasoning + tool use) carries
        // forward. First cycle for a new (room, agent) pair has no
        // prior session — chat handler will mint one and we capture
        // the returned id for next time.
        let prior_session = self.prior_session(&request.room_id, &request.agent);

        let qualified_chat = format!("{}.chat", request.agent);
        let prompt = render_agent_prompt(
            &request.agent,
            &request.agents,
            request.cycle,
            request.max_cycles,
            &request.transcript,
            request.topic.as_deref(),
            request.assigned_role.as_deref(),
        );
        let mut chat_args = json!({
            "prompt": prompt,
        });
        if let Some(sid) = prior_session.as_deref() {
            chat_args["session_id"] = json!(sid);
        }

        // In-process dispatch through the daemon's shared Axon
        // LocalRuntime. Going through `support::local_invoke` would open
        // a fresh IPC connection back to the daemon while the original
        // request is still in flight.
        let registry = self.dispatch_registry_handle.get().ok_or_else(|| {
            "internal_error: dispatch registry handle not yet set when \
             mission.discuss_round invoked"
                .to_string()
        })?;
        // Wrap the chat call in catch_unwind so an `eprintln!` to a
        // closed stderr (broken pipe → panic in the std macros) does
        // not take down our orchestration thread. The chat handler
        // does heavy fd juggling — spawning child processes, dup'ing
        // stdin/out/err for the LLM subprocess — and on rare paths
        // its `eprintln!` progress lines can panic when the parent
        // shell's stderr is no longer reachable. Catching the panic
        // and surfacing it as a typed error keeps the cycle's other
        // agents unaffected and surfaces a clean error envelope to
        // the operator.
        let response_or_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            registry.invoke_rpc_json(&qualified_chat, chat_args)
        }));
        let response = match response_or_panic {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return Err(format!("{e}")),
            Err(panic_payload) => {
                let msg = if let Some(s) = panic_payload.downcast_ref::<&'static str>() {
                    (*s).to_string()
                } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "non-string panic payload from chat handler".to_string()
                };
                return Err(format!("chat handler panicked: {msg}"));
            }
        };

        // Capture the (possibly newly minted) session_id for next
        // cycle. Driver may echo the caller's id (resume) or mint a
        // fresh one (first turn); either way it's correct to remember.
        if let Some(returned_sid) = response
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            self.remember_session(&request.room_id, &request.agent, returned_sid);
        }

        let reply = response
            .get("reply")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if reply.is_empty() || reply.eq_ignore_ascii_case("[SKIP]") {
            Ok(AgentCycleOutcome::Skip)
        } else {
            Ok(AgentCycleOutcome::Speak(reply.to_string()))
        }
    }
}

#[derive(Debug)]
struct AgentCycleRequest {
    room_id: String,
    agent: String,
    agents: Vec<String>,
    cycle: u32,
    max_cycles: u32,
    transcript: String,
    topic: Option<String>,
    assigned_role: Option<String>,
}

#[derive(Debug)]
enum AgentCycleOutcome {
    Speak(String),
    Skip,
}

// ── Prompt construction ─────────────────────────────────────────

fn render_transcript(turns: &[crate::runtime::execution::discuss::DiscussTurn]) -> String {
    if turns.is_empty() {
        return "(no turns yet)".to_string();
    }
    let mut out = String::new();
    for t in turns {
        out.push_str(&format!("[{}]: {}\n", t.speaker, t.message));
    }
    out
}

/// Render the per-(agent, cycle) prompt. Cycle 1 forces a
/// grounding pass (each agent must articulate their reading of
/// the topic, the key uncertainty, the role they want to play,
/// and whether this is a discussion or a task). Cycle 2+ is
/// recursive — agents see prior cycles' transcript and push the
/// conversation forward without repeating.
///
/// `assigned_role` is an operator-supplied override. When
/// `Some(role)` cycle 1 skips the self-nomination block and
/// tells the agent the role is already chosen by the operator;
/// `None` lets the agent self-nominate.
///
/// The prompt is intentionally long for cycle 1 — getting agents
/// to actually ground (rather than skip straight to a verdict)
/// is the load-bearing property of the whole discussion model.
fn render_agent_prompt(
    self_name: &str,
    all_agents: &[String],
    cycle: u32,
    max_cycles: u32,
    transcript: &str,
    topic: Option<&str>,
    assigned_role: Option<&str>,
) -> String {
    let peers: Vec<String> = all_agents
        .iter()
        .filter(|a| a.as_str() != self_name)
        .cloned()
        .collect();
    let peers_clause = if peers.is_empty() {
        "You are the only agent in this room — your job is mostly to think out \
         loud for the human."
            .to_string()
    } else {
        format!(
            "Other agents in this room: {}. They may speak in the same cycle as you, but \
             you cannot see their cycle-{cycle} reply when crafting yours — you only see the \
             transcript snapshot below. Their cycle-{cycle} replies will appear in the \
             transcript on cycle {next}.",
            peers.join(", "),
            next = cycle + 1
        )
    };
    let topic_clause = match topic {
        Some(t) => format!("Topic / request from the human:\n  {t}\n\n"),
        None => String::new(),
    };

    // Common rules, applied to every cycle.
    let house_rules = "\
        House rules (apply every cycle):\n\
        - This conversation may be a DISCUSSION (we exchange views, help the human think) or a \
          TASK (we have to produce something concrete: code, a file edit, a command run). You \
          decide which it is by reading the request — but until ALL participants have agreed on \
          a plan, NO ONE may use side-effecting tools (Bash, Write, Edit, EAL execution, \
          easynet.run, easynet.invoke against state-mutating abilities). Plan first, act after \
          consensus.\n\
        - Skipping is honourable. If you have nothing new to contribute this cycle, reply with \
          `[SKIP]` alone — exactly that token, single line, nothing else. Don't pad with \
          filler. A cycle in which every agent skips ends the sub-turn cleanly.\n\
        - Don't repeat what someone already said. If you fully agree, write `+1 to <speaker>` \
          and add ONE concrete extension (a counter-example, a sharpened formulation, a \
          consequence) — or skip.\n\
        - Speak as yourself in the first person. Do NOT prefix your reply with `[<name>]`; the \
          system tags speakers automatically.\n\
        - Don't address the human directly until consensus or until the human re-enters. The \
          human is observing this sub-turn but does not participate inside it.\n";

    let cycle_block = if cycle == 1 {
        let role_block = match assigned_role {
            Some(role) => format!(
                "Your role for this discussion is FIXED by the human: {role}.\n\
                 In your grounding above, briefly say how you'll fulfil this role and \
                 what you specifically bring to it. Don't re-negotiate the role.\n",
            ),
            None => "\
                Self-nominate a role: based on your strengths and what this conversation \
                seems to need, declare what role you intend to play. Pick one and explain in \
                one sentence why. Examples: critic (challenge weak claims), builder (propose \
                concrete designs / code), synthesiser (find the through-line across views), \
                domain expert (bring specific factual knowledge), skeptic (red-team the \
                premise), facilitator (keep the conversation honest), generalist (no fixed \
                role). If a peer claims the same role, that's fine — argue why YOU should \
                hold it, or pick a different one.\n"
                .to_string(),
        };
        format!(
            "This is the OPENING cycle (cycle 1 of at most {max_cycles}). Do not jump to a \
             verdict, do not propose a final answer, do not run any tool. Your only job this \
             cycle is GROUNDING. Do these four things, in this order, in your reply:\n\n\
             1. **My reading of the request.** In one or two sentences, restate what you \
                think the human is actually asking — what they want, what they care about, \
                what's in / out of scope. Different participants almost always read the same \
                request differently; surfacing your reading is the load-bearing step.\n\n\
             2. **Key uncertainty / hardest part.** What's the one thing you find most \
                non-obvious or tricky about this request? Not the solution — the problem.\n\n\
             3. **Your role.** {role_block}\n\
             4. **Mode call.** Is this a DISCUSSION or a TASK? If TASK, do you think the \
                request is clear enough to start planning, or do we need more grounding from \
                the human first? Be explicit: \"I read this as DISCUSSION/TASK; I think we \
                CAN/CANNOT start planning yet.\"\n\n\
             Keep it tight (4–8 sentences total across the four items is plenty). The point \
             is to get every participant's frame on the table so cycle 2 can build on real \
             differences instead of imagined agreement. If after writing all of the above you \
             genuinely have nothing else to add, you may end with [SKIP] (but you must still \
             complete the four items above).\n",
            role_block = role_block.trim_end(),
        )
    } else {
        // cycle >= 2 — push forward, don't repeat, can act once consensus.
        format!(
            "This is cycle {cycle} of at most {max_cycles}. Read the transcript:\n\
             - Look at every agent's grounding from cycle 1 (their reading, their stated \
               uncertainty, their declared role, their mode call). Note any disagreement \
               about whether this is discussion or task — that has to settle before any \
               execution.\n\
             - If there's a clear consensus on the plan AND the mode call was TASK, you may \
               execute (run a tool, post a result). Your reply for this cycle should then \
               include both what you did and what came of it.\n\
             - If consensus has not formed, push the discussion forward. Pick the strongest \
               disagreement or the biggest unresolved uncertainty and address it. Build on \
               peers' points using `+1 to <speaker>: <extension>` when you agree, or state \
               your concrete objection.\n\
             - You declared a role in cycle 1 (or the human assigned one). Stay in that role \
               unless you explicitly hand it off.\n\
             - If you would only repeat someone, [SKIP].\n",
        )
    };

    format!(
        "You are `{self_name}`, one participant in a multi-agent room.\n\n\
         {topic_clause}\
         {peers_clause}\n\n\
         {house_rules}\n\
         {cycle_block}\n\
         === TRANSCRIPT (snapshot at start of this cycle) ===\n\
         {transcript}\
         === END TRANSCRIPT ===\n",
    )
}

// ── Discovery surfaces ──────────────────────────────────────────

pub fn discuss_round_description() -> &'static str {
    "Run one sub-turn of a multi-agent discussion. Each cycle every \
     listed agent is queried in parallel against the same transcript \
     snapshot; an agent may speak (its reply is posted to the room \
     and visible on the discuss.subscribe stream) or skip (reply is \
     `[SKIP]`). Termination: every agent skips a cycle, or `max_cycles` \
     reached. Returns the cycle log, the speakers each cycle, and any \
     per-agent errors that occurred. Per-(room, agent) chat sessions \
     persist history across sub-turns. The CLI surface (`easynet \
     mission discuss …`) wraps this for human-bracketed turns."
}

pub fn discuss_round_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["room_id", "agents"],
        "properties": {
            "room_id": { "type": "string" },
            "agents":  {
                "type":     "array",
                "items":    { "type": "string" },
                "minItems": 1
            },
            "max_cycles": {
                "type":    "integer",
                "minimum": 1,
                "maximum": 100
            },
            "topic": { "type": "string" },
            "roles": {
                "type": "object",
                "description": "Optional operator-pinned role per agent. \
                                Shape: { \"<agent_name>\": \"<role description>\" }. \
                                Agents named here skip self-nomination in cycle 1 \
                                and stay in the assigned role through the sub-turn.",
                "additionalProperties": { "type": "string" }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_transcript_returns_placeholder_when_empty() {
        assert!(render_transcript(&[]).contains("no turns"));
    }

    #[test]
    fn render_transcript_prefixes_each_speaker() {
        use crate::runtime::execution::discuss::DiscussTurn;
        let turns = vec![DiscussTurn {
            sequence: 0,
            timestamp_unix_ms: 0,
            speaker: crate::core::domain::AgentId::new("alice"),
            message: "hi there".to_string(),
            payload: None,
        }];
        let s = render_transcript(&turns);
        assert!(s.contains("[alice]"));
        assert!(s.contains("hi there"));
    }

    #[test]
    fn agent_prompt_lists_other_agents_and_respects_self_exclusion() {
        let p = render_agent_prompt(
            "claude",
            &[
                "claude".to_string(),
                "codex".to_string(),
                "alice".to_string(),
            ],
            1,
            5,
            "(no turns yet)\n",
            Some("test topic"),
            None,
        );
        assert!(p.contains("`claude`"));
        assert!(p.contains("codex"));
        assert!(p.contains("alice"));
        // Skip protocol must be taught explicitly so the agent
        // emits exactly `[SKIP]` when passing.
        assert!(p.contains("[SKIP]"));
        // Topic clause must be present.
        assert!(p.contains("test topic"));
    }

    #[test]
    fn agent_prompt_handles_solo_discussion() {
        let p = render_agent_prompt(
            "claude",
            &["claude".to_string()],
            1,
            5,
            "(no turns yet)\n",
            None,
            None,
        );
        assert!(p.contains("only agent"));
    }

    #[test]
    fn cycle_1_prompt_forces_grounding_and_self_role_nomination() {
        // The load-bearing property of cycle 1: every agent must
        // ground (state their reading + uncertainty), nominate a
        // role, and call discussion-vs-task. A regression that
        // dropped any of these would let agents skip straight to
        // a verdict without aligning frames.
        let p = render_agent_prompt(
            "claude",
            &["claude".to_string(), "codex".to_string()],
            1,
            5,
            "(no turns yet)\n",
            Some("design a logger"),
            None,
        );
        assert!(p.contains("OPENING cycle"));
        assert!(p.contains("My reading of the request"));
        assert!(p.contains("Key uncertainty"));
        assert!(p.contains("Self-nominate a role"));
        assert!(p.contains("Mode call"));
        assert!(p.contains("DISCUSSION") && p.contains("TASK"));
        // House rules must teach plan-first for tasks.
        assert!(p.contains("Plan first"));
    }

    #[test]
    fn cycle_1_prompt_skips_self_nomination_when_role_assigned() {
        // When the operator pinned the role via --role, the
        // grounding still runs but agent does not re-negotiate
        // the role. This protects the operator's choice from
        // agent role drift.
        let p = render_agent_prompt(
            "claude",
            &["claude".to_string(), "codex".to_string()],
            1,
            5,
            "(no turns yet)\n",
            Some("design a logger"),
            Some("skeptic"),
        );
        assert!(
            p.contains("FIXED by the human"),
            "must announce the operator-pinned role: {p}"
        );
        assert!(p.contains("skeptic"));
        // Grounding still required even when role is pinned.
        assert!(p.contains("My reading of the request"));
        // Self-nomination block must NOT appear.
        assert!(!p.contains("Self-nominate a role"));
    }

    #[test]
    fn cycle_2_prompt_pushes_forward_and_allows_execution_on_consensus() {
        // Cycle 2+ relaxes grounding (already done) and unlocks
        // execution iff consensus + task mode. The prompt must
        // teach both: plan-first stays the rule until consensus,
        // execution becomes available after.
        let p = render_agent_prompt(
            "claude",
            &["claude".to_string(), "codex".to_string()],
            2,
            5,
            "(transcript)\n",
            Some("topic"),
            None,
        );
        assert!(p.contains("cycle 2"));
        // Cycle 1's grounding scaffold must NOT be repeated in cycle 2.
        assert!(!p.contains("OPENING cycle"));
        assert!(!p.contains("Self-nominate a role"));
        // Forward-push and execution gating both present.
        assert!(p.contains("consensus"));
        assert!(p.contains("execute"));
        // House rules carry into every cycle.
        assert!(p.contains("[SKIP]"));
        assert!(p.contains("Plan first"));
    }

    #[test]
    fn skip_protocol_recognises_canonical_token() {
        // The canonical form: `[SKIP]` exactly.
        assert!(matches!(
            classify_reply_for_test("[SKIP]"),
            AgentCycleOutcome::Skip
        ));
    }

    #[test]
    fn skip_protocol_recognises_case_insensitive_token() {
        assert!(matches!(
            classify_reply_for_test("[skip]"),
            AgentCycleOutcome::Skip
        ));
    }

    #[test]
    fn skip_protocol_recognises_whitespace_padded_token() {
        // Trim before matching — Claude/Codex sometimes append a
        // trailing newline.
        assert!(matches!(
            classify_reply_for_test("  [SKIP]\n"),
            AgentCycleOutcome::Skip
        ));
    }

    #[test]
    fn skip_protocol_recognises_empty_reply() {
        assert!(matches!(
            classify_reply_for_test(""),
            AgentCycleOutcome::Skip
        ));
        assert!(matches!(
            classify_reply_for_test("   \n  "),
            AgentCycleOutcome::Skip
        ));
    }

    #[test]
    fn skip_protocol_does_not_misclassify_explanatory_text() {
        // A reply that mentions [SKIP] but in prose is NOT a skip
        // — only an exact-trim match is.
        match classify_reply_for_test("I considered replying with [SKIP] but I do have a point.") {
            AgentCycleOutcome::Speak(s) => assert!(s.contains("considered")),
            _ => panic!("must classify prose containing [SKIP] as Speak"),
        }
    }

    /// Mirror the classification path of `run_agent_cycle` so we
    /// can pin the skip protocol without spinning up the IPC
    /// stack. This duplicates two lines of logic — acceptable for
    /// the testability win.
    fn classify_reply_for_test(reply: &str) -> AgentCycleOutcome {
        let trimmed = reply.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("[SKIP]") {
            AgentCycleOutcome::Skip
        } else {
            AgentCycleOutcome::Speak(trimmed.to_string())
        }
    }
}
