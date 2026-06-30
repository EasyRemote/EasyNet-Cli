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

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::core::agent_id::{AbilityName, AgentId};

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

/// A single ability invocation against an agent or device.
///
/// Previously the only variant of an `IrStep` flat struct; now the
/// `Call` variant of the `IrStep` enum (see below). The struct itself
/// is unchanged — the serde-untagged enum wrapper keeps the wire shape
/// byte-identical for existing Call-only missions so that
/// `scripts/trace-parity.sh` does not flag a spurious diff when
/// control-flow blocks (`loop` / `chat` / `handoff`) are merely
/// *possible* in the IR. A fixture refresh is only required when an
/// EAL source actually uses the new blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrCall {
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
    /// Argument key → source binding name. Stored as a `BTreeMap` so the
    /// serialized JSON has stable, lexicographic key order: every
    /// compile of the same EAL source produces a byte-identical IR,
    /// which lets `easynet mission run --emit-ir` output be diffed and
    /// makes any future "compile cache by content hash" path correct
    /// by construction. A `HashMap` here would have given the same
    /// runtime semantics but a different JSON ordering on every run.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub input_refs: BTreeMap<String, String>,
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

/// A step in a mission IR. v1 had only `Call`; v2 (PR-10, per
/// `docs/rfc/eal-control-flow-v1.md`) adds `Loop` for declarative
/// iteration. `chat` and `handoff` block forms were proposed in the
/// RFC's Draft revision but removed in the approved revision
/// (RFC §10); this enum therefore carries two variants only.
///
/// `#[serde(untagged)]` on the enum preserves the on-disk shape for
/// Call-only missions — that JSON is indistinguishable from the pre-
/// PR-10 flat struct, so existing `--emit-ir` consumers and the
/// trace-parity fixture do not see a schema break unless the mission
/// actually uses a `loop` block.
///
/// **Read ordering**: serde tries variants top-down. `Loop` carries
/// a required discriminator key (`"loop"` via `#[serde(rename = "loop")]`)
/// that a plain Call object never has, so disambiguation is
/// structural and unambiguous.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IrStep {
    /// `loop { body { … } verify { … } }` block. See RFC §3.1.
    Loop(IrLoop),
    /// Flat ability invocation. The wire shape here matches pre-PR-10
    /// v1. Listed LAST so the tagged variant above claims its
    /// object first during deserialization.
    Call(IrCall),
}

impl IrStep {
    /// `Some(&IrCall)` when this step is a flat invocation; `None`
    /// for the block variants. Used by tests that walk the IR
    /// expecting flat call shape.
    #[cfg(test)]
    pub fn as_call(&self) -> Option<&IrCall> {
        match self {
            IrStep::Call(c) => Some(c),
            IrStep::Loop(_) => None,
        }
    }

    /// Every `IrCall` reachable inside this step, in source order.
    /// A `Loop` flattens its `body` and `verify` into the iteration.
    /// A `Call` step yields exactly itself. Useful for static
    /// analyses that need to walk every leaf call.
    pub fn walk_calls<'a>(&'a self, out: &mut Vec<&'a IrCall>) {
        match self {
            IrStep::Call(c) => out.push(c),
            IrStep::Loop(l) => {
                for s in &l.body {
                    s.walk_calls(out);
                }
                for s in &l.verify {
                    s.walk_calls(out);
                }
            }
        }
    }

    /// Worst-case cross-agent call count contributed by this step.
    /// Used by the RFC §4.1 planner-time bound check.
    ///
    /// - `Call` contributes 1.
    /// - `Loop` contributes `max_iters * (|body-calls| + |verify-calls|)`.
    pub fn static_call_bound(&self) -> u64 {
        match self {
            IrStep::Call(_) => 1,
            IrStep::Loop(l) => {
                let mut body_calls = Vec::new();
                for s in &l.body {
                    s.walk_calls(&mut body_calls);
                }
                let mut verify_calls = Vec::new();
                for s in &l.verify {
                    s.walk_calls(&mut verify_calls);
                }
                (l.max_iters as u64).saturating_mul((body_calls.len() + verify_calls.len()) as u64)
            }
        }
    }
}

/// Body of an `IrStep::Loop` variant. Kept as a standalone struct so
/// serde can name the shape and the `walk_calls` / `static_call_bound`
/// helpers above can hold typed references without pattern-match
/// gymnastics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrLoop {
    /// Serde discriminator — absent from `IrCall`, so the untagged
    /// enum disambiguates by structural shape. Value is always the
    /// literal `"loop"`; kept as a field (not derived) so a hand-
    /// written fixture can include it unambiguously.
    #[serde(rename = "loop")]
    pub kind: IrLoopTag,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub max_iters: u32,
    pub body: Vec<IrStep>,
    pub verify: Vec<IrStep>,
    /// Export `<name>.result`. `None` when the loop is anonymous —
    /// no binding leaks to the enclosing scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_binding: Option<String>,
}

/// Singleton tag type — serializes to the literal string `"loop"`.
/// Exists only to give serde a discriminator on the untagged enum.
/// Manual Serialize/Deserialize lives in the `tag_serde` module
/// below — don't add serde derives here, they would duplicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrLoopTag;

/// Archival EAL `emit` record lowered into Mission IR.
///
/// Emits are intentionally not `IrStep`s: they do not dispatch, retry,
/// produce receipts, or alter phase scheduling. The interpreter resolves
/// them from captured call outputs at mission completion and copies the
/// resolved records into `ExecutionTrace::emissions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrEmit {
    pub name: String,
    pub kind: String,
    pub value: IrEmitValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IrEmitValue {
    Literal { value: serde_json::Value },
    Binding { binding: String },
}

/// Serializer helper for the singleton `IrLoopTag`. Serializes to
/// the fixed wire string `"loop"`; a different value on
/// deserialization is a hard error. Chat / Handoff tag serdes lived
/// here in the Draft-revision scaffold and were removed with their
/// variants in the approved RFC.
mod tag_serde {
    use super::IrLoopTag;
    use serde::de::{self, Visitor};
    use serde::{Deserializer, Serializer};
    use std::fmt;

    macro_rules! tag_impl {
        ($ty:ty, $lit:literal) => {
            impl serde::Serialize for $ty {
                fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    s.serialize_str($lit)
                }
            }
            impl<'de> serde::Deserialize<'de> for $ty {
                fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                    struct V;
                    impl<'de> Visitor<'de> for V {
                        type Value = $ty;
                        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                            write!(f, "the literal string {:?}", $lit)
                        }
                        fn visit_str<E: de::Error>(self, v: &str) -> Result<$ty, E> {
                            if v == $lit {
                                Ok(<$ty>::default())
                            } else {
                                Err(E::custom(format!("expected {:?}, got {:?}", $lit, v)))
                            }
                        }
                    }
                    d.deserialize_str(V)
                }
            }
            impl Default for $ty {
                fn default() -> Self {
                    Self
                }
            }
        };
    }

    tag_impl!(IrLoopTag, "loop");
}

fn default_ct() -> String {
    "application/json".into()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IrFailurePolicy {
    #[default]
    Continue,
    Abort,
    Skip,
    Retry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionIr {
    pub name: String,
    pub steps: Vec<IrStep>,
    pub phases: Vec<PhaseRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emits: Vec<IrEmit>,
    #[serde(default)]
    pub constraints: IrConstraints,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IrConstraints {
    pub max_parallel: i32,
    pub deadline_seconds: i64,
}

impl IrConstraints {
    /// RFC §4.1 default cap on a mission's worst-case static call
    /// count. Enforced at compile time by the planner. Held as a
    /// `const fn` rather than a field so the bound is unambiguous
    /// and does not need per-mission configuration — a mission that
    /// genuinely needs >256 static calls is one that should be
    /// split at the mission boundary, not one that should tune a
    /// dial on a single mission.
    pub const fn default_max_calls() -> u64 {
        256
    }
}
