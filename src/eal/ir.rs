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
/// `docs/rfc/eal-control-flow-v1.md`) adds `Loop`, `Chat`, and
/// `Handoff` for declarative control flow.
///
/// `#[serde(untagged)]` on the enum preserves the on-disk shape for
/// Call-only missions — that JSON is indistinguishable from the pre-
/// PR-10 flat struct, so existing `--emit-ir` consumers and the
/// trace-parity fixture do not see a schema break unless the mission
/// actually uses a block form.
///
/// **Read ordering**: serde tries variants top-down. `Loop` / `Chat`
/// / `Handoff` carry required discriminator keys (`"loop"` /
/// `"chat"` / `"handoff"` via `#[serde(rename = "...")]`) that a
/// plain Call object never has, so disambiguation is structural and
/// unambiguous.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IrStep {
    /// `loop { body { … } verify { … } }` block. See RFC §3.1.
    Loop(IrLoop),
    /// `chat { participants … }` block. See RFC §3.2.
    Chat(IrChat),
    /// `handoff { from … to … }` block. See RFC §3.3.
    Handoff(IrHandoff),
    /// Flat ability invocation. The wire shape here matches pre-PR-10
    /// v1. Listed LAST so the tagged variants above claim their
    /// objects first during deserialization.
    Call(IrCall),
}

impl IrStep {
    /// `Some(&IrCall)` when this step is a flat invocation; `None`
    /// for the block variants. Used by planner passes that only care
    /// about flat call shape (e.g. the worst-case call-count bound
    /// walks the tree and treats each `Call` leaf as one invocation).
    pub fn as_call(&self) -> Option<&IrCall> {
        match self {
            IrStep::Call(c) => Some(c),
            _ => None,
        }
    }

    /// Every `IrCall` reachable inside this step, in source order.
    /// Block variants flatten their `body` / `verify` / participant
    /// list into the iteration. A `Call` step yields exactly itself.
    /// Useful for static analyses that need to walk every leaf call.
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
            IrStep::Chat(_) | IrStep::Handoff(_) => {
                // Chat and Handoff do not nest IrSteps — their
                // participant / from-to shape is flat. Their
                // dispatch-time call count is derived from
                // `max_turns * participants` or the single handoff
                // call respectively; callers that need a count use
                // `static_call_bound` below.
            }
        }
    }

    /// Worst-case cross-agent call count contributed by this step.
    /// Used by the RFC §4.1 planner-time bound check.
    ///
    /// - `Call` contributes 1.
    /// - `Loop` contributes `max_iters * (|body-calls| + |verify-calls|)`.
    /// - `Chat` contributes `max_turns * participants.len()`.
    /// - `Handoff` contributes 1 (summary mode adds 1 for the
    ///   implicit `from.summarize` call; see the impl).
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
                (l.max_iters as u64)
                    .saturating_mul((body_calls.len() + verify_calls.len()) as u64)
            }
            IrStep::Chat(c) => {
                (c.max_turns as u64).saturating_mul(c.participants.len() as u64)
            }
            IrStep::Handoff(h) => match h.context_mode {
                HandoffContextMode::Summary => 2,
                _ => 1,
            },
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

/// Body of `IrStep::Chat`. RFC §3.2 governs the fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrChat {
    #[serde(rename = "chat")]
    pub kind: IrChatTag,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub participants: Vec<AgentId>,
    pub max_turns: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(default)]
    pub visibility: ChatVisibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_binding: Option<String>,
}

/// Singleton tag type — serializes to the literal string `"chat"`.
/// Manual Serialize/Deserialize lives in the `tag_serde` module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrChatTag;

/// Body of `IrStep::Handoff`. RFC §3.3 governs the fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrHandoff {
    #[serde(rename = "handoff")]
    pub kind: IrHandoffTag,
    pub from: AgentId,
    pub to: AgentId,
    pub context_mode: HandoffContextMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_binding: Option<String>,
}

/// Singleton tag type — serializes to the literal string `"handoff"`.
/// Manual Serialize/Deserialize lives in the `tag_serde` module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrHandoffTag;

/// RFC §3.2 `visibility` enum. `fan_out` is the default (every
/// participant sees every other participant's prior turns).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatVisibility {
    #[default]
    FanOut,
    RoundRobin,
}

/// RFC §3.3 / §4.6 `context_mode` enum.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HandoffContextMode {
    /// Pack the source agent's last run response verbatim. No
    /// implicit summarize call; the receiving agent gets the full
    /// transcript body as `prompt:`.
    Full,
    /// Ask the source agent to summarize its own session via
    /// `from.summarize(of: session)` before handoff. The summary
    /// (bounded 4 KiB) becomes the receiving agent's `prompt:`.
    #[default]
    Summary,
    /// No context packing — the receiving agent sees only the
    /// static `prompt:` attribute declared on the handoff block.
    None,
}

/// Serializer helpers for the singleton tag types. We want each of
/// them to serialize to a fixed string on the wire (`"loop"` /
/// `"chat"` / `"handoff"`) without manually implementing
/// `Serialize`/`Deserialize` against every possible bikeshed. The
/// trick: use `#[serde(with = "...")]` would be nicer, but
/// hand-rolling once is cheaper and more auditable. These impls
/// live below so they stay next to the types they serve.
mod tag_serde {
    use super::{IrChatTag, IrHandoffTag, IrLoopTag};
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
    tag_impl!(IrChatTag, "chat");
    tag_impl!(IrHandoffTag, "handoff");
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
