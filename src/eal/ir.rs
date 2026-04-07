// EasyNet CLI — Mission IR v2
// ===========================
//
// File: src/eal/ir.rs
// Description: Serializable intermediate representation — the compilation target of EAL.
//
// Key Innovation (over Axon MissionControl v1):
//   Steps have explicit `input_refs` (arg_key → source binding) and `output_binding`
//   (variable name for captured result). This enables data flow between steps,
//   which MissionControl v1 cannot do (it discards InvokeResponse.result).
//
// Execution Targets:
//   - Client-side interpreter (current — temporary engine)
//   - MissionControl v2 (future — server-side, stateful, with result persistence)
//   - JSON inspection via `easynet mission run --emit-ir`
//
// Proto Upgrade Path:
//   MissionStep += { output_binding (field 13), input_refs (field 14) }
//   StepStatus  += { result (field 10), result_content_type (field 11) }
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use crate::shared::agent_id::{AbilityName, AgentId};

/// Where a step dispatches to. The two variants encode the ontological
/// distinction between agent (network actor, ontology §6.4) and device
/// (hosting substrate, ontology §5). The runtime dispatcher matches on
/// this enum and never re-classifies by string lookup. See
/// `docs/AGENT_IDENTITY.md` invariant 2 ("no `is_agent` string check").
///
/// EAL surface invariant (LOAD-BEARING):
///
///   member-call form (agent.ability) is the ONLY way to invoke an agent.
///   traditional call form (call ... on ...) is STRICTLY device-only.
///   No implicit agent fallback is allowed.
///
/// Lowering map (set by parser, executed by planner — never by runtime):
///
///   `claude.chat(prompt: "hi")`           → `IrTarget::Agent(AgentId)`
///   `call "x" on "node-1" with {...}`     → `IrTarget::Device { node_id }`
///   `call "x" on "<registered-agent>"`    → REJECTED at
///                                            `run_mission_inproc` time
///                                            with a clear error pointing
///                                            at member-call form. See
///                                            `cli/mission_runs.rs::find_implicit_agent_fallback`
///                                            and `no_implicit_agent_fallback_*` tests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IrTarget {
    /// Address a registered agent by its logical identity. Lowered
    /// from EAL member-call form `agent.ability(...)`.
    Agent(AgentId),
    /// Address a hosting substrate by node id. Lowered from EAL
    /// traditional form `call "ability" on "node-id"`.
    Device { node_id: String },
}

impl IrTarget {
    /// One-line display form for trace summaries and CLI banners. The
    /// canonical form for agents is `<tenant>/<name>` (full form, see
    /// `docs/AGENT_IDENTITY.md` §3.4); for devices it is the bare node id.
    pub fn display_string(&self) -> String {
        match self {
            Self::Agent(id) => id.to_string(),
            Self::Device { node_id } => node_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrStep {
    pub step_id: String,
    pub step_name: String,

    /// The ability (method name) being invoked on `target`. **Not** an
    /// identity type; see `docs/AGENT_IDENTITY.md` §10 for why this is
    /// `AbilityName` and not `AbilityRef`.
    pub ability: AbilityName,

    /// Resolved dispatch target. Set at planner-time from the EAL
    /// surface form's `TargetKind`; never re-classified at runtime.
    pub target: IrTarget,

    pub static_arguments: serde_json::Value,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub input_refs: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_binding: Option<String>,

    #[serde(default)]
    pub timeout_seconds: i32,
    #[serde(default)]
    pub max_retries: i32,
    #[serde(default)]
    pub on_failure: IrFailurePolicy,
    #[serde(default)]
    pub optional: bool,
    #[serde(default = "default_ct")]
    pub content_type: String,
}

fn default_ct() -> String { "application/json".into() }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IrFailurePolicy { #[default] Continue, Abort, Skip, Retry }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionIr {
    pub name: String,
    pub steps: Vec<IrStep>,
    pub phases: Vec<PhaseRange>,
    #[serde(default)]
    pub constraints: IrConstraints,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseRange { pub start: usize, pub end: usize }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IrConstraints {
    pub max_parallel: i32,
    pub deadline_seconds: i64,
}
