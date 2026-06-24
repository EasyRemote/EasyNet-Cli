// EasyNet CLI — System Abilities (`system.*` namespace)
// ======================================================
//
// File: src/runtime/system/mod.rs
// Description: Device-level abilities published by `easynet-daemon`.
//              Distinct from agent abilities (which live under
//              `runtime::abilities` and bind to one registered AI
//              agent), system abilities belong to the *node*
//              itself: ping, schedule, session-attach, permission,
//              discuss, loop. Their handlers run inside the daemon,
//              not inside an agent subprocess.
//
// Naming
// ------
// All system abilities are named `system.<feature>[.<verb>]`. Today
// only `observe.health` exists; PR-ATTACH onwards extends the namespace.
//
// Per-feature module layout
// -------------------------
// One file per feature (PR-ATTACH adds `session_ability.rs`,
// PR-PERM adds `permission_ability.rs`, etc.). Each file exports
// (a) the schema/manifest helpers and (b) a registration function
// that mounts the handler on the `AxonAbilityCatalog`.
//
// CI rule (`scripts/check-dispatch-boundary.sh`)
// ----------------------------------------------
// Handler functions in this directory MUST NOT inspect
// `self.node_id` or `target_node` to decide locality. The stage-1
// resolver in `runtime::invocation_target` is the only place that
// makes that decision; handlers consume `InvocationTarget` and act
// on it.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod a2a_bridge_ability;
pub mod a2a_client_ability;
/// ability.publish + ability.unpublish — root meta-abilities that
/// let a curator session (spawned by mission.think) materialise a
/// new ability into a registered agent's abilities/ directory, or
/// delete an existing one. Sibling of skill.publish (Phase 3); the
/// two surfaces are how judge-validated experience is sunk back
/// into the ability/skill ontology.
pub mod ability_publish_ability;
/// Generator for the `abilities/system/<name>.ability.toml`
/// files. Single source of truth: every TOML descriptor on
/// disk is the output of `render_ability_toml(name,
/// description_for(name), input_schema_for(name))`. The drift
/// test in this module's `tests` block enforces the equality;
/// the `gen-ability-tomls` binary regenerates after any code
/// change to the metadata.
pub mod ability_toml;
/// terminal.attach — interactive bidirectional terminal
/// stream over InvokeBidi. The seventh and final member of the
/// AXIOM Tier 2.5 Baseline Locomotion Profile (the streaming
/// counterpart to process.exec / shell.run). See
/// `runtime/execution/pty/` for the underlying PtyService.
pub mod admin_status_ability;
pub mod agent_lifecycle_ability;
pub mod agent_list_ability;
/// RFC-006-C v0.1 — `<user>.api_key.{create,list,revoke}` ability
/// family for OpenAI-compatibility bearer tokens. Independent of
/// the RFC-002 keyring vault (different threat model: bearer
/// capability vs cryptographic identity).
pub mod api_key_ability;
/// RFC-012 §RemoteWebSurface — `browser.{open_session,
/// send_input, capture_viewport, close_session}` ability family.
/// v0 mock handlers (no real WebView); RFC-013 W1–W8 replaces the
/// mock with wry per platform.
pub mod browser_session_ability;
pub mod chat_ability;
pub mod chat_history_ability;
pub mod context_ability;
pub mod context_loaders;
pub mod device_ability_registrar;
pub mod device_ability_store;
pub mod device_ops_ability;
pub mod discover_ability;
pub mod discuss_ability;
/// eal_executor — backs `[exec] kind = "eal"` on agent-authored
/// ability manifests. Lets an ability's implementation be a small
/// EAL program that composes other abilities. Sibling of
/// shell_executor; both consume `template` for `{{ var }}`
/// substitution before dispatching.
pub mod eal_executor;
/// device-hosted node/ability operations: list_nodes, describe_node,
/// remove_node, deploy_ability, uninstall_ability. The CLI side
/// (`easynet device …`, `easynet ability deploy / uninstall`)
/// reaches these through `support::local_invoke::invoke_local_ability`,
/// the same path every other CLI surface uses.
pub(crate) mod federation_probe;
/// fs.transfer — bidirectional chunked file upload /
/// download. Pairs with the EasyNet backend's
/// /api/v1/files/{upload,download} HTTP routes; one signed
/// InvokeBidi session per transfer. Atomic write (staging +
/// rename), SHA-256 over content, 1 GiB byte cap.
pub mod file_transfer_ability;
/// RFC-006-B v0.6 — Pages reference system. The first reference
/// realisation of the Resource Execution Model: publishing a
/// folder of static bytes as a website. Owns the
/// `<user>.pages.{publish,unpublish,list,get}` and
/// `<user>.<project_id>.page.fetch` ability families.
pub mod files;
/// AXIOM §"Tier 2.5" Baseline Locomotion Profile, filesystem
/// half. Three abilities (`fs.read`, `fs.write`, `fs.list`)
/// published by every host-embodied agent claiming the
/// `baseline-locomotion-v1` profile.
pub mod fs_ability;
/// fs.edit — surgical string-replace primitive over a single
/// text file. Default contract: old_string MUST occur exactly
/// once; ambiguous matches reject with the count rather than
/// silently rewriting all occurrences. Pass replace_all=true
/// to opt into bulk replacement. Empty old_string + missing
/// target = create-new-file. Atomic write (tempfile +
/// fdatasync + rename) shared with fs.write.
pub mod fs_edit_ability;
/// http.request — outbound HTTP client. Last member of the
/// Baseline Locomotion Profile. Issues one request per call,
/// captures status / headers / body up to a cap, redacts
/// auth-bearing headers (Authorization, Cookie, X-API-Key, …)
/// from every receipt the auditor may persist. Schemes
/// restricted to http / https; CR/LF in header values
/// rejected; redirect / timeout / body caps enforced.
pub mod host_stream_executor;
pub mod http_executor;
pub mod http_request_ability;
pub mod invocation_history_ability;
pub mod invoke_ability;
/// RFC-005 v3.2 A9 — `meta.list_resources`. Resource discovery
/// surface for the eight media abilities above. Reads
/// `~/.easynet/resources.json` via `persistence::resources` and
/// projects to the wire shape; no device backend needed, so it
/// ships fully working.
pub mod list_resources_ability;
pub mod loop_ability;
pub mod mcp_bridge_ability;
pub mod mcp_client_ability;
pub mod mcp_executor;
pub mod mcp_reflective_registry;
/// Real (non-stub) media handlers, swapped in over the
/// `media_abilities` stubs one ability at a time. PR3a delivers
/// the `camera.snapshot` vertical slice with a deterministic
/// synthetic backend; PR3 lands cpal/nokhwa/screen.
pub mod media;
/// RFC-005 v3.2 A1–A8 — eight physical-channel media abilities
/// (mic.subscribe, camera.subscribe/snapshot, screen.subscribe/
/// snapshot, speaker.publish, voice.subscribe, voice.transcribe).
/// PR2 ships every handler as a stub that enforces
/// INV-SUBJECT-ENVELOPE; PR3 / PR3a swap individual stubs out for
/// real implementations.
pub mod media_abilities;
pub mod meta_ability;
pub mod mission_ability;
pub mod network_health_ability;
/// RFC-006-C v0.1 — `openai.{chat_completions,list_models}`
/// adapter abilities. The OpenAI streaming protocol becomes a
/// transport view over chat-base ability dispatch sequences.
pub mod openai_compat_ability;
/// mission.discuss / mission.think — round-robin and think-act-
/// observe orchestration abilities. The `easynet mission discuss`
/// and `easynet mission think` CLI subcommands invoke these,
/// keeping a single ability-only path for both EAL programs and
/// the operator CLI.
pub mod orchestration_ability;
pub mod pages;
pub mod permission_ability;
pub mod ping;
pub mod plugin_lifecycle_ability;
/// AXIOM §"Tier 2.5" Baseline Locomotion Profile,
/// structured-execution member. `process.exec` spawns one
/// process via OS-level argv (NO shell interpretation);
/// for pipes / redirects / glob a caller uses `shell.run`.
/// The eight-stage shell pipeline is NOT run here; the
/// structured input shape is the security boundary.
pub mod process_exec_ability;
pub mod profiles;
pub mod pty_attach_ability;
/// terminal.input / terminal.read /
/// terminal.resize — unary-RPC data plane. Used by the
/// EasyNet backend's PTYDriver before the WS bidi optimisation.
/// Mutually exclusive with pty_attach_ability per session: the
/// reader thread takes one fd dup, attach takes another, and two
/// readers on the same PTY race for incoming bytes.
pub mod pty_io_ability;
/// terminal.create / terminal.close — control-
/// plane lifecycle for PtyService sessions. attach (above) is
/// the data-plane sibling.
pub mod pty_lifecycle_ability;
/// Per-ability real-invocation tests. Each `#[test]` exercises
/// one published ability through the catalogue's test-only invoke
/// probe with realistic args (not `{}`). This is the test layer that
/// validates the full chain: registry lookup + handler
/// invocation + service interaction + response shape. See the
/// file's preamble for what "real" does and does NOT cover.
#[cfg(test)]
mod real_invoke_tests;
pub mod schedule_ability;
pub mod session_ability;
/// shell_executor — backs `[exec] kind = "shell"` on agent-authored
/// ability manifests. Distinct from `shell_run_ability` (the
/// daemon-owned `shell.run` baseline locomotion ability): this
/// module is invoked from the dispatch path when a manifest pins a
/// concrete argv contract, bypassing chat translation. See module
/// docstring for substitution model + injection-safety argument.
pub mod shell_executor;
/// shell.run — the shell-interpreted member of the Baseline
/// Locomotion Profile. Takes a bash command STRING (with pipes
/// / redirects / glob), runs it through the AXIOM Tier 2.5
/// 8-stage pipeline (ast → security → permissions →
/// pathconstraints → readonly → destructive), and on full pass
/// dispatches to `bash -c <command>` via the SAME runner
/// process.exec uses. See AXIOM Tier 2.5 §"shell.run / 8-stage
/// pipeline" for the normative spec; the pipeline lives in
/// `support/shellguard/`, this module is the thin
/// agent-dispatch wiring on top.
pub mod shell_run_ability;
pub mod skill_ability;
pub mod skill_install_ability;
/// skill.publish + skill.unpublish + skill.list — sibling of
/// ability_publish_ability. Where ability.publish sinks a
/// generally-useful experience as a published ability (device-
/// visible), skill.publish sinks an agent-private experience as a
/// skill (lives in the agent's own skill pool). The judge's
/// `value_kind` field picks between the two.
pub mod skill_publish_ability;
pub mod teach_ability;
/// `{{ var }}` template substitution shared by every executor that
/// consumes `[exec]`-bound ability manifests (shell argv, EAL
/// source, …). Pulled out so the substitution model — including
/// missing-arg / unclosed-brace error shapes — has one canonical
/// implementation and one test surface.
pub mod template;
/// mission.think — long-running worker+judge orchestration. Lets a
/// task outgrow Claude Code's per-session ~200-step limit by
/// resuming the worker's session_id across cycles, with an
/// independent judge session emitting a memory-classification
/// verdict per cycle (consumed by the Phase 5 curator).
pub mod think_ability;
/// voice.* call signaling abilities backing the `easynet call …`
/// subcommand surface (create, show, join, leave, end, watch,
/// report_metrics). v1 stores call state in-process; persistence
/// + federation fan-out land with the RFC-006 follow-up.
pub mod voice_call_ability;

// ─── Split out of this file (F-027 / T4.5): assembly & exports only ───
//
// The implementation lives one concern per file; this module keeps
// the declarations and re-exports so every external path
// (`crate::runtime::agents::build_registry`, `PagesIdentity`,
// `published_abilities`, …) is unchanged.
pub(crate) mod catalog_metadata;
mod pages_identity;
mod registry_builder;

#[cfg(test)]
mod assembly_tests;

pub use catalog_metadata::*;
pub use pages_identity::*;
pub use registry_builder::*;
