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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrStep {
    pub step_id: String,
    pub step_name: String,
    pub function_name: String,
    pub target_node_id: String,

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
