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
// only `device.observe.health` exists; PR-ATTACH onwards extends the namespace.
//
// Per-feature module layout
// -------------------------
// One file per feature (PR-ATTACH adds `session_ability.rs`,
// PR-PERM adds `permission_ability.rs`, etc.). Each file exports
// (a) the schema/manifest helpers and (b) a registration function
// that mounts the handler on the `LocalAbilityRegistry`.
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
/// device.terminal.attach — interactive bidirectional terminal
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
/// RFC-012 §RemoteWebSurface — `device.browser.{open_session,
/// send_input, capture_viewport, close_session}` ability family.
/// v0 mock handlers (no real WebView); RFC-013 W1–W8 replaces the
/// mock with wry per platform.
pub mod browser_session_ability;
pub mod chat_ability;
pub mod context_loaders;
pub mod device_ops_ability;
pub mod discover_ability;
pub mod discuss_ability;
/// eal_executor — backs `[exec] kind = "eal"` on agent-authored
/// ability manifests. Lets an ability's implementation be a small
/// EAL program that composes other abilities. Sibling of
/// shell_executor; both consume `template` for `{{ var }}`
/// substitution before dispatching.
pub mod eal_executor;
/// device-hosted node/ability/remote operations: list_nodes, describe_node,
/// remove_node, deploy_ability, uninstall_ability, exec_remote,
/// register_self, deregister_self. The CLI side
/// (`easynet device …`, `easynet ability deploy / uninstall / exec`)
/// reaches these through `support::local_invoke::invoke_local_ability`,
/// the same path every other CLI surface uses.
pub(crate) mod federation_probe;
/// device.fs.transfer — bidirectional chunked file upload /
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
/// RFC-006-C v0.1 — `01HUB.openai.{chat_completions,list_models}`
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
pub mod policy_ability;
/// AXIOM §"Tier 2.5" Baseline Locomotion Profile,
/// structured-execution member. `process.exec` spawns one
/// process via OS-level argv (NO shell interpretation);
/// for pipes / redirects / glob a caller uses `shell.run`.
/// The eight-stage shell pipeline is NOT run here; the
/// structured input shape is the security boundary.
pub mod process_exec_ability;
pub mod profiles;
pub mod pty_attach_ability;
/// device.terminal.input / device.terminal.read /
/// device.terminal.resize — unary-RPC data plane. Used by the
/// EasyNet backend's PTYDriver before the WS bidi optimisation.
/// Mutually exclusive with pty_attach_ability per session: the
/// reader thread takes one fd dup, attach takes another, and two
/// readers on the same PTY race for incoming bytes.
pub mod pty_io_ability;
/// device.terminal.create / device.terminal.close — control-
/// plane lifecycle for PtyService sessions. attach (above) is
/// the data-plane sibling.
pub mod pty_lifecycle_ability;
/// Per-ability real-invocation tests. Each `#[test]` exercises
/// one published ability through the live dispatcher with
/// realistic args (not `{}`). This is the test layer that
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

use std::sync::Arc;

use crate::registry::agents::AgentRegistry;
use crate::runtime::ability_dispatch::LocalAbilityRegistry;
use crate::runtime::execution::discuss::DiscussService;
use crate::runtime::execution::loop_instance::LoopService;
use crate::runtime::execution::permission::PermissionService;
use crate::runtime::execution::pty::PtyService;
use crate::runtime::execution::schedule::ScheduleService;
use crate::runtime::execution::session::SessionService;

/// Identity + configuration the registry build needs to mint
/// user-rooted ability families (pages, files, api_key). The
/// `<user>` slot is sourced explicitly so the registry build is
/// pure — no global env-var reads, no thread-locals, no implicit
/// dependence on credentials.json. Production callers
/// (`bin/easynet-daemon.rs`, supervisor reboots) read
/// EASYNET_PAGES_USER + credentials.json once at boot and pass
/// the resolved value here. Tests that exercise the user-rooted
/// surface pass a fixed username; unpaired-daemon tests pass
/// `None` and the family stays unregistered.
#[derive(Debug, Clone, Default)]
pub struct PagesIdentity {
    /// Username segment for `<user>.api_key.*` / `<user>.pages.*` /
    /// `<user>.files.*`. `None` means "this daemon isn't paired
    /// yet" — the user-rooted families are skipped (no
    /// `self.api_key.*` placeholder leak).
    pub user: Option<String>,
    /// Realm the user-rooted handlers stamp into URAs. Defaults
    /// to `crate::ura::REALM_EASYNET`.
    pub realm: Option<String>,
    /// HTTP listener port for the pages server. `None` falls back
    /// to the historical 8787.
    pub listener_port: Option<u16>,
}

impl PagesIdentity {
    /// Resolve from the boot-time env vars. Read ONCE at process
    /// startup; downstream callers receive the resolved struct.
    /// Tests should not call this — they construct
    /// `PagesIdentity` directly so the registry shape is
    /// deterministic regardless of process env state.
    pub fn from_env() -> Self {
        let user = std::env::var("EASYNET_PAGES_USER")
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(|| {
                crate::persistence::config::load_credentials()
                    .ok()
                    .and_then(|c| c.username)
                    .filter(|v| !v.is_empty())
            });
        Self {
            user,
            realm: std::env::var("EASYNET_PAGES_REALM")
                .ok()
                .filter(|v| !v.is_empty()),
            listener_port: std::env::var("EASYNET_PAGES_PORT")
                .ok()
                .and_then(|s| s.parse::<u16>().ok()),
        }
    }
}

/// Build a `LocalAbilityRegistry` populated with every v1 system
/// ability handler. Suitable for early-boot smoke tests + the
/// `published_ability_names` helper that the discovery publisher
/// consumes. Tests get fresh empty sub-services and an empty agent
/// registry; the daemon bin calls `build_registry_with_services`
/// instead with its real Kernel handles + loaded agents.
///
/// **No env-var read**: the registry shape is determined by the
/// arguments alone. Tests that don't care about the user-rooted
/// surface pass a default `PagesIdentity` (user = None) and get
/// a deterministic catalogue regardless of any leaked env state.
pub fn build_registry() -> Arc<LocalAbilityRegistry> {
    build_registry_with_services(
        Arc::new(SessionService::new()),
        Arc::new(PermissionService::new()),
        Arc::new(DiscussService::new()),
        Arc::new(ScheduleService::new()),
        Arc::new(LoopService::new()),
        None,
        &AgentRegistry::default(),
        Arc::new(Vec::new()),
        PagesIdentity::default(),
    )
}

/// Build a `LocalAbilityRegistry` with sub-service handles wired
/// in. The daemon bin calls this with the Kernel's actual handles
/// at boot; tests construct a fresh registry per case.
///
/// `agents` and `loaders` were added when chat became a first-class
/// system-registered ability: a `<agent>.chat` handler is registered
/// per agent (see `chat_ability::register`). `loaders` is the seam
/// for pluggable context loaders — empty in v1, populated in
/// subsequent PRs without touching the daemon's startup code.
pub fn build_registry_with_services(
    sessions: Arc<SessionService>,
    perms: Arc<PermissionService>,
    discuss: Arc<DiscussService>,
    schedule: Arc<ScheduleService>,
    loop_svc: Arc<LoopService>,
    invocation_ledger: Option<Arc<easynet_axon::invocation::InvocationLedger>>,
    agents: &AgentRegistry,
    loaders: Arc<Vec<Arc<dyn chat_ability::ContextLoader>>>,
    pages_identity: PagesIdentity,
) -> Arc<LocalAbilityRegistry> {
    let mut reg = LocalAbilityRegistry::new();
    ping::register(&mut reg);
    network_health_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion Profile, filesystem
    // half. Three stateless handlers (fs.read / fs.write /
    // fs.list) — every host-embodied agent claiming
    // `baseline-locomotion-v1` MUST expose them.
    fs_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion — surgical text
    // edit. Sibling of fs.read / fs.write; uses the SAME
    // atomic-write path (tempfile + fdatasync + rename) so
    // the crash-resilience story is uniform.
    fs_edit_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion Profile —
    // structured execution. `process.exec` shares the
    // destructive command list and process-execution
    // hardening (tempfile-backed output, tree-kill on
    // timeout, env defaults) with `shell.run` via the
    // `support::shellguard` subsystem.
    process_exec_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion Profile —
    // shell-interpreted execution. `shell.run` is the only
    // member of the profile that takes a bash command STRING;
    // the 8-stage shellguard pipeline (ast → security →
    // permissions → pathconstraints → readonly → destructive)
    // gates every dispatch.
    shell_run_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion — HTTP client.
    // Last member of the seven-ability profile; first-class
    // surface for outbound network so receivers can audit
    // every external call uniformly instead of going through
    // a shell.run-wrapped curl.
    http_request_ability::register(&mut reg);
    // device.invocation.* — read-only audit/history surfaces over
    // the Axon invocation ledger. The ledger is written by the
    // gRPC invocation service; these handlers only expose persisted
    // URA-complete records for UI/backend tracing.
    invocation_history_ability::register(&mut reg, invocation_ledger);
    // AXIOM §"Tier 2.5" Baseline Locomotion — PTY data-plane and
    // its lifecycle control-plane. device.terminal.create /
    // device.terminal.close manage the session catalog;
    // device.terminal.attach pumps stdin/stdout bidirectionally
    // over InvokeBidi for interactive workloads (REPLs, editors,
    // text-mode TUI). All three share one process-wide PtyService
    // (single Arc, lazy init): a session created by …_create
    // is the same session …_attach pumps and …_close tears down,
    // so the three abilities cohere even though they're three
    // separate handlers.
    let pty = Arc::new(PtyService::new());
    let pty_io = pty_io_ability::PtyIoService::new();
    pty_lifecycle_ability::register(&mut reg, Arc::clone(&pty), Some(pty_io.clone()));
    pty_attach_ability::register(&mut reg, Arc::clone(&pty));
    // device.terminal.input / _read / _resize — unary-RPC data
    // plane. The backend's PTYDriver invokes these for the
    // production HTTP-session terminal flow before the WebSocket
    // bidi optimisation kicks in. Sharing the PtyService Arc with
    // the lifecycle + attach handlers means a session created by
    // …_create is reachable through all three surfaces (unary,
    // bidi, lifecycle) — operators choose one mode per session.
    pty_io_ability::register(&mut reg, pty, pty_io);
    // device.fs.transfer — bidi chunked file upload/download.
    // Pairs with the EasyNet backend's /api/v1/files/{upload,
    // download} routes. No shared service state needed; the
    // handler opens its own per-session FS handle on each
    // OpenBidi.
    file_transfer_ability::register(&mut reg);
    // RFC-005 v3.2 — eight physical-channel media abilities (A1–A8)
    // plus meta.list_resources (A9). The eight A1–A8 stubs are
    // registered first; PR3a then swaps individual entries
    // (currently `camera.snapshot`) for real envelope-aware
    // handlers via `media::*::register`. The order matters: the
    // real-handler register MUST come after the stub register so
    // its envelope-aware variant takes precedence at dispatch time
    // (the registry stores stub + env-aware handlers separately
    // and the dispatcher's "envelope-first" lookup picks env-aware
    // when both are present).
    media_abilities::register(&mut reg);
    media::camera_snapshot::register(&mut reg);
    media::screen_snapshot::register(&mut reg);
    media::mic_subscribe::register(&mut reg);
    list_resources_ability::register(&mut reg);
    // device.agent.start / device.agent.stop — Invoke-side mirror
    // of `easynet agent add/remove`. LLM sub-agents are registry
    // rows (not resident processes), so start ≡ insert into
    // ~/.easynet/agents.json and return the canonical URA;
    // stop ≡ delete the row (idempotent).
    agent_lifecycle_ability::register(&mut reg);
    // device-hosted node/ability/remote operations (list_nodes, describe_node,
    // remove_node, deploy_ability, uninstall_ability, exec_remote,
    // register_self, deregister_self). These are the canonical
    // ability surfaces backing the CLI's device + ability subcommands.
    device_ops_ability::register(&mut reg);
    // device.browser.* — RFC-012 §RemoteWebSurface; v0 mock
    // handlers per RFC-013 plan. capture_viewport is a streaming
    // verb; the other three are unary RPC.
    browser_session_ability::register(&mut reg);
    // voice.* call signaling abilities — `easynet call …`
    // subcommand surface routes through these via the same
    // ability-only invocation path every other CLI surface uses.
    voice_call_ability::register(&mut reg);
    // policy.{evaluate,simulate} — admission-gate consumer surface
    // pinned to the §A6 contract. v1 is allow-all; the gate's
    // rewiring to actually call this ability lands in a follow-up
    // (see policy_ability module preamble).
    policy_ability::register(&mut reg);
    session_ability::register(&mut reg, sessions);
    permission_ability::register(&mut reg, perms);
    discuss_ability::register(&mut reg, Arc::clone(&discuss));
    schedule_ability::register(&mut reg, schedule);
    loop_ability::register(&mut reg, loop_svc);
    // The shared OnceLock seam consumed by every ability that needs
    // to dispatch through the live registry post-boot:
    // mcp.bridge.call_tool, a2a.bridge.send_task, meta.list_abilities,
    // per-agent <agent>.invoke, and the dynamic fallback resolver
    // installed by chat_ability::register. Created BEFORE
    // chat_ability::register so the fallback resolver — which gains
    // the ability to synthesize `<self>.invoke` for hot-added agents
    // — can close over it. Set once after `Arc::new(reg)` below.
    let local_registry_handle: Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>> =
        Arc::new(std::sync::OnceLock::new());
    // mission.discuss_round — sub-turn orchestration ability.
    // The CLI `easynet mission discuss …` and any EAL caller drive
    // multi-agent discussions through this name. Shares the
    // DiscussService with the discuss.* triple (same room state)
    // and consumes the shared registry handle so per-cycle
    // <agent>.chat invocations stay in-process — going through
    // IPC would deadlock the daemon's accept loop.
    orchestration_ability::register(
        &mut reg,
        Arc::clone(&discuss),
        Arc::clone(&local_registry_handle),
    );
    chat_ability::register(
        &mut reg,
        agents,
        loaders,
        Arc::clone(&local_registry_handle),
    );
    // RFC-006-B v0.6 — Pages reference system. Chained AFTER
    // chat_ability so its fallback resolver is consulted FIRST
    // (chain_rpc_fallback prepends). The fallback handles
    // `<user>.pages.{publish,unpublish,list,get}` and
    // `<user>.<project>.page.fetch`. The default user identity is
    // sourced from the daemon's published self-identity; for the
    // MVP we accept the env var `EASYNET_PAGES_USER` so the demo
    // and tests can pin a deterministic user without reaching
    // into the keyring resolver. Listener port comes from
    // `EASYNET_PAGES_PORT` (default 8787).
    {
        // User-rooted ability families (`<user>.api_key.*`,
        // `<user>.pages.*`, `<user>.files.*`). Identity sourced
        // explicitly from the `pages_identity` argument — no
        // env-var read here.
        //
        // M5 of the system-namespace migration banned the `<self>`
        // placeholder; an unpaired daemon (`pages_identity.user`
        // is None) skips the user-rooted family entirely. The
        // ability surface returns once pairing completes and the
        // supervisor rebuilds the registry with a populated
        // identity.
        if let Some(user) = pages_identity.user.clone() {
            let realm = pages_identity
                .realm
                .clone()
                .unwrap_or_else(|| crate::ura::REALM_EASYNET.to_string());
            let listener_port = pages_identity.listener_port.unwrap_or(8787);
            let pages_realm = realm.clone();
            pages::register(
                &mut reg,
                pages::PagesConfig {
                    user: user.clone(),
                    realm,
                    listener_port,
                },
                Arc::clone(&local_registry_handle),
            );
            // Files reference system: content-addressed blob store
            // serving `/v1/files{,/<id>/content}` + chat-multimodal
            // URA dereferences. Same `<user>` identity as pages so
            // one user owns both surface families.
            files::register(
                &mut reg,
                files::FilesConfig {
                    user: user.clone(),
                    realm: pages_realm,
                },
            );
            // RFC-006-C v0.1 — API key abilities. Register under the
            // same `user` identity pages used so a single user owns
            // both surface families on this daemon.
            api_key_ability::register(&mut reg, &user);
        }
        // RFC-006-C v0.1 — device-local OpenAI shim. Device-owned,
        // no `<user>` slot — registers regardless of pairing state.
        openai_compat_ability::set_dispatch_handle(Arc::clone(&local_registry_handle));
        openai_compat_ability::set_identity(pages_identity.clone());
        openai_compat_ability::register(&mut reg);
    }
    skill_install_ability::register(&mut reg);
    // ability.publish + ability.unpublish — root meta-abilities. See
    // module preamble for trust model and on-disk layout. Stateless
    // handlers (no captured registry handle), so order vs other
    // registrations is irrelevant.
    ability_publish_ability::register(&mut reg);
    // skill.publish + skill.unpublish + skill.list/file operations —
    // sibling of ability_publish_ability. Same statelessness; same
    // order independence.
    skill_publish_ability::register(&mut reg);
    // mission.think — long-running worker+judge orchestration.
    // Consumes the shared dispatch_registry_handle so per-cycle
    // <agent>.chat invocations stay in-process; same rationale as
    // mission.discuss_round (going back through the IPC client
    // would deadlock the daemon's accept loop).
    think_ability::register(&mut reg, Arc::clone(&local_registry_handle));
    // mcp.bridge.{list_tools, call_tool} — MCP edge adapter.
    //
    // list_tools projects local AbilityDescriptors to the MCP
    // tools/list shape. Provider runs on every call so a daemon
    // restart that picks up a freshly-canonicalised URA (or a
    // future hot-add of a hosted Agent) is reflected without re-
    // registering the handler. `load_host_descriptors` is the same
    // recipe the MCP stdio server uses, so an external MCP client
    // and an in-process Invoke caller see one catalog.
    //
    // call_tool dispatches an in-process Invoke against the named
    // local ability. The shared OnceLock seam (declared above
    // before chat_ability::register) is the chicken-and-egg fix:
    // every consumer needs an `Arc` to the registry being built,
    // but the registry isn't yet wrapped in an `Arc` at
    // registration time. Set the lock once after `Arc::new(reg)`
    // completes; every closure's `get()` returns the populated
    // handle.
    mcp_bridge_ability::register(
        &mut reg,
        profiles::load_host_descriptors,
        Arc::clone(&local_registry_handle),
    );
    // mission.run — single ability surface for EAL execution. The
    // canonical orchestration entry point referenced by AGENTS.md
    // ("cross-agent calls go through the mission runtime; there is
    // no second path"). Without this an LLM inside an agent had to
    // shell out to `easynet mission run`, which depended on shell
    // access and bypassed the in-process dispatcher's invariants.
    //
    // Registered BEFORE meta.list_abilities so the live-registry
    // merge inside that handler picks up the mission entry point
    // — otherwise the LLM's discovery flow would not see this
    // ability and would fall back to fabricating answers.
    mission_ability::register(&mut reg);
    // Per-agent self-bundle: `<agent>.discover` and `<agent>.invoke`.
    //
    // Why here and not inside `chat_ability::register`: invoke needs
    // the dispatch registry handle (`local_registry_handle` above) to
    // resolve the target ability through the live registry — including
    // entries registered AFTER chat_ability runs (mission.run,
    // meta.list_abilities, the dynamic fallback resolver). The handle
    // is in scope here, so wiring sits next to the other consumers
    // (mcp_bridge / meta / a2a_bridge).
    //
    // Both handlers close over a snapshot of `agents`: `<agent>.discover`
    // enumerates peer manifests for scope-filtered candidates;
    // `<agent>.invoke` validates that the requested `target` is a known
    // local agent. The snapshot misses brand-new `easynet agent add`
    // entries until the next daemon restart — same caveat that applies
    // to chat_ability's dynamic-fallback snapshot, and tracked by the
    // same future "runtime.refresh_local_tools" follow-up.
    for agent_name in agents.agents.keys() {
        let snapshot_for_discover = agents.clone();
        discover_ability::register_for_agent(
            &mut reg,
            agent_name.clone(),
            move || snapshot_for_discover.clone(),
            Arc::clone(&local_registry_handle),
        );
        let snapshot_for_invoke = agents.clone();
        invoke_ability::register_for_agent(
            &mut reg,
            agent_name.clone(),
            move || snapshot_for_invoke.clone(),
            Arc::clone(&local_registry_handle),
        );
    }

    // RFC-002 §3.3: register `device.keyring.*` for the daemon's
    // own self-bundle, scoped under the literal owner `device`.
    // The daemon publishes its 10 keyring abilities under this
    // namespace so any local agent can call them through the
    // standard dispatch path. Auto-init the on-disk store when
    // absent — passphrase comes from EASYNET_KEYRING_PASS or
    // falls back to a fixed deterministic local pass for the
    // local-fast default. Failures here MUST NOT block daemon
    // boot; we log the error and skip keyring registration. The
    // resolver layer copes with absence by treating every URA
    // as Unknown.
    //
    // The legacy owner string was `<self>` — a "this device"
    // alias. v4.1.5 onward names the actor explicitly: keyring
    // belongs to the device, so the owner is `device`. The
    // catalogue now lists these as `device.keyring.<verb>`,
    // matching the URA `callee = device/<id>` that already
    // covers them.
    //
    // EASYNET_KEYRING_DISABLE=1 skips auto-init entirely. Tests
    // that don't want side effects on the user's real keyring
    // file set this; production daemons leave it unset.
    if std::env::var("EASYNET_KEYRING_DISABLE").is_err() {
        match init_keyring_for_daemon() {
            Ok(handle) => {
                crate::runtime::keyring::abilities::register_for_owner(&mut reg, "device", handle);
            }
            Err(e) => {
                let err_msg = format!("{e}");
                crate::op_event!(
                    component = device_keyring,
                    kind = auto_init_failed,
                    level = "warn",
                    error = err_msg,
                );
            }
        }
    }
    // meta.{describe,list_abilities} — Agent self-introspection on
    // the same descriptor catalogue PLUS the live registry. describe
    // is the lightweight identity+summary surface; list_abilities
    // merges the static profile catalogue with everything currently
    // registered in `reg` (mission.run, per-agent <agent>.<verb>
    // verbs, hot-reloaded TOMLs) so a discover-then-invoke flow sees
    // every callable name. Visibility filtering per §1.6 happens at
    // the admission gate, not here.
    meta_ability::register(
        &mut reg,
        profiles::load_host_descriptors,
        Arc::clone(&local_registry_handle),
        pages_identity.user.clone(),
    );
    // a2a.bridge.list_skills — same edge-adapter pattern as the MCP
    // bridge above, but for the A2A agent-card surface. Closes over
    // a clone of the AgentRegistry passed in here. v1 has no
    // hot-reload of `agents.json`, so the snapshot stays accurate
    // for the daemon's lifetime; the closure is still cheap to call.
    let agents_for_a2a = agents.clone();
    a2a_bridge_ability::register(
        &mut reg,
        move || crate::registry::agents::load_agents().unwrap_or_else(|_| agents_for_a2a.clone()),
        Arc::clone(&local_registry_handle),
    );
    // a2a.client.send_task — outbound A2A. Reads through a process-
    // wide DISPATCHER_HANDLE that the daemon bin populates after
    // building the AbilityDispatcher (see
    // a2a_client_ability::set_dispatcher). Tests leave the lock
    // unset; the handler returns ok:false on every call, which is
    // what the unit tests verify.
    a2a_client_ability::register(&mut reg);
    // mcp.client.{list,call} — outbound MCP. Boots an
    // McpClientService from ~/.easynet/mcp_clients.json (missing
    // file → empty service, no upstreams). Each upstream MCP
    // server is spawned lazily on first call; subsequent calls
    // reuse the live connection. Parse errors at boot bubble up
    // because a malformed file is an operator typo, not a "no
    // upstreams" condition.
    let mcp_clients_path =
        crate::runtime::execution::mcp_client::McpClientService::default_config_path();
    let mcp_client_svc =
        match crate::runtime::execution::mcp_client::McpClientService::from_path(&mcp_clients_path)
        {
            Ok(svc) => Arc::new(svc),
            Err(e) => {
                let path_display = format!("{}", mcp_clients_path.display());
                let err_msg = format!("{e}");
                crate::op_event!(
                    component = mcp_client,
                    kind = config_load_failed,
                    level = "warn",
                    path = path_display,
                    error = err_msg,
                    fallback = "empty_service",
                );
                Arc::new(crate::runtime::execution::mcp_client::McpClientService::new())
            }
        };
    mcp_client_ability::register(&mut reg, mcp_client_svc.clone());

    // Install the same `Arc<McpClientService>` as the process-wide
    // handle used by `[exec] kind="mcp"` ability dispatch. Before this
    // line `mcp_executor::run_mcp_exec` would return a typed error;
    // after this line every MCP surface in the daemon — outbound
    // `device.mcp.client.*`, reflective registry below, and exec —
    // shares one connection pool, one config snapshot, one `next_id`
    // sequence per upstream. No silent divergence between surfaces.
    crate::runtime::agents::mcp_executor::set_process_client(mcp_client_svc.clone());

    // Reflectively register every tool of every configured upstream
    // MCP server as a first-class ability. After this call, the
    // outbound `device.mcp.client.call` ability is still available
    // (for callers who want the explicit-server-name shape), but
    // operators + MCP-Bench can also reach each tool by its bare
    // ability name (e.g. `weather_lookup` instead of
    // `device.mcp.client.call({server, name, ...})`).
    //
    // The function is async because tools/list is a JSON-RPC RPC;
    // build_registry_with_services is sync. We bridge through
    // `reflect_mcp_upstreams_sync`. Failures are LOGGED, not panicked:
    // a broken upstream MCP server must not block daemon boot — every
    // other ability stays available and the operator sees per-server
    // failures in stderr.
    //
    // **Identity invariant.** The owner URA for reflected abilities
    // is the mcp-profile agent under the daemon's paired user. An
    // unpaired daemon has no `pages_identity.user` and we therefore
    // SKIP reflective registration entirely — we will not fabricate
    // a synthetic `user_id = "device"` to mint an agent URA, because
    // per AGENT_IDENTITY.md §2 ("identity, not locator") that would
    // forge an agent identity that no `easynet:///r/.../user/...`
    // backs. The outbound `device.mcp.client.*` family remains
    // available so operators can still reach upstream tools through
    // the explicit-server-name shape; only the bare-name projection
    // is gated on a paired user.
    // Boot-time reflection state we carry across the Arc::new(reg)
    // boundary so the hot-reload sinks attached afterwards know which
    // names they own per server. `None` here means "reflection didn't
    // run" — the unpaired-daemon arm below — and the sink attachment
    // below short-circuits in that case.
    let mut reflection_per_server: Option<(
        String,
        std::collections::BTreeMap<String, Vec<String>>,
    )> = None;

    if let (Some(user_slot), realm_slot) = (
        pages_identity.user.clone(),
        pages_identity
            .realm
            .clone()
            .unwrap_or_else(|| easynet_axon::ura::REALM_EASYNET.to_string()),
    ) {
        let mcp_owner_ura = easynet_axon::ura::agent_ura(&realm_slot, &user_slot, "mcp");
        match reflect_mcp_upstreams_sync(&mcp_client_svc, &mut reg, &mcp_owner_ura) {
            Ok(report) => {
                if !report.registered.is_empty() || !report.failed.is_empty() {
                    let registered_count = report.registered.len();
                    let failed_count = report.failed.len();
                    crate::op_event!(
                        component = mcp_reflective,
                        kind = reflection_summary,
                        registered = registered_count,
                        failed = failed_count,
                    );
                    for f in &report.failed {
                        let server = f.server.as_str();
                        let tool = f.tool.as_deref().unwrap_or("(server)");
                        let reason = f.reason.as_str();
                        crate::op_event!(
                            component = mcp_reflective,
                            kind = tool_skipped,
                            server = server,
                            tool = tool,
                            reason = reason,
                        );
                    }
                }
                // Aggregate per-server reflected names so the
                // hot-reload sinks know the starting set and can
                // diff against future `tools/list_changed` pushes.
                let mut per_server: std::collections::BTreeMap<String, Vec<String>> =
                    std::collections::BTreeMap::new();
                for r in &report.registered {
                    per_server
                        .entry(r.server.clone())
                        .or_default()
                        .push(r.ability_name.clone());
                }
                reflection_per_server = Some((mcp_owner_ura.clone(), per_server));
            }
            Err(e) => {
                let err_msg = format!("{e}");
                crate::op_event!(
                    component = mcp_reflective,
                    kind = reflection_skipped,
                    level = "warn",
                    reason = "runtime_bridge_failed",
                    error = err_msg,
                );
            }
        }
    } else {
        // Unpaired daemons emit a single informational line so an
        // operator who configured `mcp_clients.json` but forgot to
        // pair a user understands why their MCP tools are not
        // showing up as bare-name abilities. We do NOT consult the
        // service for its server count here — that requires the
        // async lock, and this code path runs in the sync boot
        // context — so the log is unconditional. False positives
        // (printing this line when there are no servers configured
        // either) are cheap; false silences would frustrate
        // operators.
        crate::op_event!(
            component = mcp_reflective,
            kind = reflection_skipped,
            reason = "daemon_unpaired",
        );
    }
    // device.agent.list — operational view of registered LLM
    // sub-agents. Cheap-row projection (name, runtime, model, label);
    // for the protocol agent-card view see a2a.bridge.list_skills.
    let agents_for_device_view = agents.clone();
    agent_list_ability::register(&mut reg, move || {
        crate::registry::agents::load_agents().unwrap_or_else(|_| agents_for_device_view.clone())
    });
    // admin.status — operator-facing component snapshot. The
    // ability-count provider reads through the same OnceLock the
    // bridge handlers use, so the count is accurate at call time
    // (post-Arc-wrap; pre-set the OnceLock returns 0 which only
    // happens during the brief window before `.set()` below).
    {
        let handle_for_admin = Arc::clone(&local_registry_handle);
        admin_status_ability::register(&mut reg, move || {
            handle_for_admin
                .get()
                .map(|r| r.list_abilities().len())
                .unwrap_or(0)
        });
    }
    let arc = Arc::new(reg);
    // Populate the shared OnceLock now that the registry is wrapped.
    // Both mcp.bridge.call_tool and a2a.bridge.send_task read through
    // it to dispatch into other local abilities; until this line runs
    // they each return isError("not initialised") on every call.
    let _ = local_registry_handle.set(Arc::clone(&arc));

    // Hot-reload sinks. Wired after Arc::new(reg) so each sink can
    // hold a `Weak<LocalAbilityRegistry>` that survives daemon
    // shutdown gracefully (the sink becomes a no-op when the registry
    // is dropped, rather than blocking shutdown by keeping a strong
    // ref). Sinks live inside `McpClientService::notification_sinks`
    // for the lifetime of the daemon process — they're never
    // explicitly unregistered, which matches the daemon's whole-
    // process lifecycle.
    if let Some((owner_ura, per_server)) = reflection_per_server.as_ref() {
        attach_mcp_refresh_sinks_sync(&mcp_client_svc, &arc, owner_ura, per_server);
    }

    arc
}

/// Daemon-side convenience wrapper. Loads the agent registry and
/// builds the full `LocalAbilityRegistry` in one call, swallowing a
/// load failure into the empty-registry case (so a brand-new install
/// without `~/.easynet/agents.json` still boots).
///
/// `loaders`:
/// * `Some(vec)` — caller-provided context-loader chain. Tests
///   pass `Some(Arc::new(Vec::new()))` to get no loaders attached.
/// * `None` — auto-attach the daemon's default chain
///   (`user_profile` + `schedule` + `memory`). This is the path
///   `easynet-daemon` boots through: it called the explicit
///   variant before slice 35, which made every library / smoke
///   caller hand-build the chain or get silently empty
///   `context_used`.
///
/// RFC-002 §3.2 keyring auto-init for the daemon. Locates the
/// keyring file at `$XDG_CONFIG_HOME/easynet/keyring.json` (or
/// platform fallback), opens it under the passphrase from
/// `EASYNET_KEYRING_PASS` env var, and falls back to a deterministic
/// local-only passphrase when none is set. The local fallback is
/// fine for the `.localhost` default — federation peers never see
/// the master key, and the file is mode 0o600.
///
/// Returns `Err` only on filesystem / decode / KDF errors; absence
/// of an existing file is the happy path (creates a fresh ring).
fn init_keyring_for_daemon(
) -> anyhow::Result<std::sync::Arc<crate::runtime::keyring::KeyringHandle>> {
    use crate::runtime::keyring::store::default_keyring_path;
    use crate::runtime::keyring::KeyringHandle;
    let path = std::env::var("EASYNET_KEYRING_PATH")
        .map(std::path::PathBuf::from)
        .ok()
        .map_or_else(|| default_keyring_path(), |p| Ok(p))?;
    let pass = std::env::var("EASYNET_KEYRING_PASS").unwrap_or_else(|_| {
        // Local-fast default. Operators wanting stronger isolation
        // set EASYNET_KEYRING_PASS to a real secret. The literal
        // here is NOT a security boundary — the threat model for
        // local-fast assumes the host filesystem is the trust
        // boundary anyway. RFC-002 §3.2.
        "easynet-local-default-passphrase-v1".into()
    });
    Ok(std::sync::Arc::new(KeyringHandle::open_or_create(
        path, &pass,
    )?))
}

/// Exists so `bin/easynet-daemon.rs` does not have to reach into the
/// `pub(crate) registry::agents` module — that module's visibility is
/// intentionally crate-private.
pub fn build_registry_for_daemon(
    sessions: Arc<SessionService>,
    perms: Arc<PermissionService>,
    discuss: Arc<DiscussService>,
    schedule: Arc<ScheduleService>,
    loop_svc: Arc<LoopService>,
    invocation_ledger: Option<Arc<easynet_axon::invocation::InvocationLedger>>,
    loaders: Option<Arc<Vec<Arc<dyn chat_ability::ContextLoader>>>>,
    pages_identity: PagesIdentity,
) -> Arc<LocalAbilityRegistry> {
    let agents = match crate::registry::agents::load_agents() {
        Ok(r) => r,
        Err(e) => {
            let err_msg = format!("{e}");
            crate::op_event!(
                component = agent_registry,
                kind = load_failed,
                level = "warn",
                error = err_msg,
                fallback = "empty_registry",
            );
            AgentRegistry::default()
        }
    };
    let loaders = loaders
        .unwrap_or_else(|| Arc::new(context_loaders::default_loaders(Arc::clone(&schedule))));
    build_registry_with_services(
        sessions,
        perms,
        discuss,
        schedule,
        loop_svc,
        invocation_ledger,
        &agents,
        loaders,
        pages_identity,
    )
}

/// Public list of every v1 system-ability *name*. Used by
/// `registry::a2a_labels` to populate the top-level
/// `system_skills[]` field of the node-roster v2 envelope so peers
/// discover what device-level abilities this daemon offers without
/// invoking anything.
///
/// The list is built from the live registry to avoid name drift
/// between the publisher and the dispatcher.
///
/// **M2 of the system-namespace migration**: at M1 every system
/// ability registered both a legacy name (e.g. `fs.read`) and a
/// canonical name (e.g. `device.fs.read`); the registry's
/// `list_abilities()` returns both. This function FILTERS to
/// canonical-only at M2 — legacy entries become hidden aliases
/// for inbound dispatch only and are no longer advertised on the
/// hub agent roster, the TOML manifest catalogue, or the LLM
/// discover surface. The `is_canonical_or_unmapped` predicate
/// retains every name whose first segment is in
/// `{device, hub, <agent-id>, <user-id>}` (canonical / per-agent
/// / per-user) and drops legacy first segments belonging to the
/// closed system-namespace set.
///
/// At M3 the legacy entries are removed entirely from the
/// registry; the filter then becomes a no-op.
pub fn published_ability_names() -> Vec<String> {
    build_registry()
        .list_abilities()
        .into_iter()
        .filter(|name| is_canonical_or_unmapped(name))
        .collect()
}

/// True iff `name` is a canonical post-M2 ability name (or a name
/// without a legacy/canonical pair, e.g. per-agent / per-user
/// verbs that always lived under their owner's prefix).
///
/// Drops the closed set of legacy first-segments that the M1
/// dual-aliasing covers — those names are still dispatchable
/// (M1 retained both bindings) but should not appear in any
/// catalogue surface that publishes "what to call". Public
/// surfaces (advertise prelude, TOML manifests, MCP bridge,
/// LLM discover) emit canonical only; legacy continues to work
/// for any caller that has them hard-coded, but isn't
/// advertised.
///
/// `M3` deletes the legacy registrations entirely; this filter
/// becomes a no-op then.
pub fn is_canonical_or_unmapped(name: &str) -> bool {
    // Legacy first-segments — closed set per M1 mapping table.
    // Names with these heads are filtered OUT (legacy alias of
    // a canonical pair).
    const LEGACY_HEADS: &[&str] = &[
        "fs", "http", "shell", "process", "observe", "admin", "easynet", "meta", "mission",
        "schedule", "loop", "discuss", "mcp", "a2a", "policy", "ability", "camera", "mic",
        "screen", "speaker", "voice", "skill", "consent", "01HUB",
    ];
    if let Some((head, _)) = name.split_once('.') {
        if LEGACY_HEADS.contains(&head) {
            return false;
        }
    }
    true
}

/// One row of a system ability's discovery + registration metadata.
///
/// Centralises (name, description, input_schema) so every consumer —
/// the federation label builder (`registry::a2a_labels`), the
/// runtime-local register publisher (`runtime::publish`), and any
/// future `easynet ability list --system` surface — pulls from one
/// table. Adding a new system ability now requires updating exactly
/// one match arm in `metadata_for`; previously the same name lived
/// in three places that could (and did) drift.
#[derive(Debug, Clone)]
pub struct SystemAbilityMetadata {
    pub name: String,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
    pub hints: crate::runtime::ability_descriptor::AbilityHints,
}

/// Every published system ability's metadata, in the deterministic
/// order `published_ability_names()` returns.
///
/// `<agent>.chat` is **excluded** even when the live registry would
/// include it: those entries are already published to the
/// axon-runtime via `runtime::publish::republish_abilities_via_advertise`
/// off the on-disk `chat.ability.toml` manifest, and re-publishing
/// them through the system path would double-register with a
/// different (synthesised) schema. Filter is by suffix because the
/// agent name varies per install.
pub fn published_abilities() -> Vec<SystemAbilityMetadata> {
    let registry = build_registry();
    registry
        .list_abilities()
        .into_iter()
        .filter(|name| is_canonical_or_unmapped(name))
        .filter(|name| !name.ends_with(".chat"))
        // RFC-002 §3.3 keyring abilities are owner-namespaced under
        // `device` and self-described by `keyring::abilities` — they
        // don't go through the system descriptor table. Filter them
        // for the same reason `<agent>.chat` is filtered: their
        // schema lives inside the registering module, not here.
        .filter(|name| !name.starts_with("device.keyring."))
        .map(|name| SystemAbilityMetadata {
            description: description_for(&name),
            input_schema: input_schema_for(&name),
            hints: discovery_hints_for(&registry, &name),
            name,
        })
        .collect()
}

pub(crate) fn discovery_hints_for(
    registry: &crate::runtime::ability_dispatch::LocalAbilityRegistry,
    name: &str,
) -> crate::runtime::ability_descriptor::AbilityHints {
    if name.ends_with(".chat") {
        // Hosted chat abilities are registered on the local stream
        // surface today, but the user-facing control-plane path still
        // serves them through unary invoke + OpenAI compatibility.
        // Advertising them as `streaming_only` would regress the UI
        // into choosing InvokeStream against a daemon path that is not
        // yet wired for generic stream fallback.
        return Default::default();
    }
    let has_rpc = registry.has_rpc(name);
    let has_stream = registry.has_stream(name);
    let has_bidi = registry.has_bidi(name);
    crate::runtime::ability_descriptor::AbilityHints {
        streaming_only: has_stream && !has_rpc && !has_bidi,
        bidi_only: has_bidi,
        ..Default::default()
    }
}

/// Human-readable description for a published system ability name.
///
/// Authoritative source for the description text. `registry::a2a_labels`
/// re-exports through this so the wire payload and the runtime
/// register call agree byte-for-byte. Falls back to a short generic
/// blurb for unknown names; the `_ if name.ends_with(".chat")` arm
/// exists because `published_ability_names()` includes per-agent chat
/// handlers when called from the daemon registry (the `published_abilities`
/// filter strips them, but other callers may not).
pub fn description_for(name: &str) -> &'static str {
    match name {
        "device.observe.health" => ping::description(),
        "device.observe.network_health" => network_health_ability::description(),
        "device.policy.evaluate" => policy_ability::evaluate_description(),
        "device.policy.simulate" => policy_ability::simulate_description(),
        "device.session.list" => session_ability::list_description(),
        "device.session.attach" => session_ability::attach_description(),
        "device.consent.subscribe" => permission_ability::subscribe_description(),
        "device.consent.decide" => permission_ability::decide_description(),
        "device.consent.list_pending" => permission_ability::list_pending_description(),
        "device.discuss.create" => discuss_ability::create_description(),
        "device.discuss.post" => discuss_ability::post_description(),
        "device.discuss.subscribe" => discuss_ability::subscribe_description(),
        "device.discuss.list_turns" => discuss_ability::list_turns_description(),
        "device.schedule.add" => schedule_ability::add_description(),
        "device.schedule.list" => schedule_ability::list_description(),
        "device.schedule.remove" => schedule_ability::remove_description(),
        "device.schedule.enable" => schedule_ability::enable_description(),
        "device.loop.create" => loop_ability::create_description(),
        "device.loop.status" => loop_ability::status_description(),
        "device.loop.subscribe" => loop_ability::subscribe_description(),
        "device.loop.cancel" => loop_ability::cancel_description(),
        "device.skill.install" => skill_install_ability::install_description(),
        "device.skill.remove" => skill_install_ability::remove_description(),
        "device.skill.upgrade" => skill_install_ability::upgrade_description(),
        "device.mcp.bridge.list_tools" => mcp_bridge_ability::list_tools_description(),
        "device.mcp.bridge.call_tool" => mcp_bridge_ability::call_tool_description(),
        "device.a2a.bridge.list_skills" => a2a_bridge_ability::list_skills_description(),
        "device.a2a.bridge.send_task" => a2a_bridge_ability::send_task_description(),
        "device.a2a.client.send_task" => a2a_client_ability::send_task_description(),
        "device.mcp.client.list" => mcp_client_ability::list_description(),
        "device.mcp.client.call" => mcp_client_ability::call_description(),
        "device.agent.list" => agent_list_ability::list_agents_description(),
        "device.meta.describe" => meta_ability::describe_description(),
        "device.meta.list_abilities" => meta_ability::list_abilities_description(),
        "device.mission.run" => mission_ability::run_description(),
        "device.mission.track" => mission_ability::track_description(),
        "device.mission.cancel" => mission_ability::cancel_description(),
        // AXIOM §"Tier 2.5" Baseline Locomotion — filesystem half.
        "device.fs.read" => fs_ability::description_read(),
        "device.fs.write" => fs_ability::description_write(),
        "device.fs.list" => fs_ability::description_list(),
        "device.fs.edit" => fs_edit_ability::description(),
        "device.process.exec" => process_exec_ability::description(),
        "device.shell.run" => shell_run_ability::description(),
        "device.http.request" => http_request_ability::description(),
        "device.invocation.history.list" => invocation_history_ability::list_history_description(),
        "device.invocation.history.get" => invocation_history_ability::get_history_description(),
        "device.invocation.trace.get" => invocation_history_ability::get_trace_description(),
        "device.terminal.create" => pty_lifecycle_ability::description_create(),
        "device.terminal.close" => pty_lifecycle_ability::description_close(),
        "device.terminal.attach" => pty_attach_ability::description(),
        "device.terminal.input" => pty_io_ability::input_description(),
        "device.terminal.read" => pty_io_ability::read_description(),
        "device.terminal.resize" => pty_io_ability::resize_description(),
        "device.fs.transfer" => file_transfer_ability::description(),
        "device.agent.start" => agent_lifecycle_ability::start_agent_description(),
        "device.agent.stop" => agent_lifecycle_ability::stop_agent_description(),
        "device.node.list" => device_ops_ability::list_nodes_description(),
        "device.node.describe" => device_ops_ability::describe_node_description(),
        "device.node.remove" => device_ops_ability::remove_node_description(),
        "device.ability.deploy" => device_ops_ability::deploy_ability_description(),
        "device.ability.uninstall" => device_ops_ability::uninstall_ability_description(),
        "device.remote.exec" => device_ops_ability::exec_remote_description(),
        "device.node.register" => device_ops_ability::register_self_description(),
        "device.node.deregister" => device_ops_ability::deregister_self_description(),
        "device.mission.discuss_round" => orchestration_ability::discuss_round_description(),
        "device.voice.create_call" => voice_call_ability::create_call_description(),
        "device.voice.show_call" => voice_call_ability::show_call_description(),
        "device.voice.join_call" => voice_call_ability::join_call_description(),
        "device.voice.leave_call" => voice_call_ability::leave_call_description(),
        "device.voice.end_call" => voice_call_ability::end_call_description(),
        "device.voice.watch_call" => voice_call_ability::watch_call_description(),
        "device.voice.report_metrics" => voice_call_ability::report_metrics_description(),
        "device.voice.list_calls" => voice_call_ability::list_calls_description(),
        // RFC-012 §RemoteWebSurface — device.browser.* family.
        "device.browser.open_session" => browser_session_ability::open_session_description(),
        "device.browser.send_input" => browser_session_ability::send_input_description(),
        "device.browser.capture_viewport" => {
            browser_session_ability::capture_viewport_description()
        }
        "device.browser.close_session" => browser_session_ability::close_session_description(),
        "device.admin.status" => admin_status_ability::description(),
        "device.ability.publish" => ability_publish_ability::publish_description(),
        "device.ability.unpublish" => ability_publish_ability::unpublish_description(),
        "device.skill.publish" => skill_publish_ability::publish_description(),
        "device.skill.unpublish" => skill_publish_ability::unpublish_description(),
        "device.skill.list" => skill_publish_ability::list_description(),
        "device.skill.tree" => skill_publish_ability::tree_description(),
        "device.skill.read_file" => skill_publish_ability::read_file_description(),
        "device.skill.write_file" => skill_publish_ability::write_file_description(),
        "device.mission.think" => think_ability::description(),
        // RFC-005 v3.2 A1–A8 — media abilities. `media_abilities`
        // owns the single source of truth (the `ABILITIES` table);
        // the projection here is one Option lookup, no per-name
        // arm. A 9th media ability requires touching only that
        // table; this arm picks the new name up automatically.
        n if media_abilities::description(n).is_some() => media_abilities::description(n).unwrap(),
        // RFC-005 v3.2 A9 — meta.list_resources. Lives in its own
        // module because the handler is fully real (not a stub).
        list_resources_ability::ABILITY_META_LIST_RESOURCES => {
            list_resources_ability::description()
        }
        // RFC-006-C v0.1 — device-local OpenAI protocol shim. The
        // handler runs on this host and only sees host-local
        // chat-base abilities; there is no hub round-trip in the
        // call path. Hub-side OpenAI adapters (if any realm hub
        // chooses to advertise them) live behind `hub.openai.*`,
        // queried through `federation.resolve` — the device daemon
        // never pre-registers a `hub.*` name.
        "device.openai.chat_completions" => {
            "OpenAI-compatible /v1/chat/completions served by the \
             device daemon. Forwards a chat-completion request to a \
             host-local chat-base ability (`<agent>.chat`) and \
             projects the streaming/non-streaming reply into \
             OpenAI's response shape."
        }
        "device.openai.list_models" => {
            "OpenAI-compatible /v1/models served by the device daemon. \
             Returns every host-local chat-base ability \
             (`<agent>.chat`) the calling identity has dispatch grants \
             on, projected as OpenAI `Model` objects."
        }
        _ if name.ends_with(".chat") => "Send a chat prompt to the locally-installed agent.",
        // `<user>.api_key.{create,list,revoke}` — user-rooted
        // credential-lifecycle abilities. `<user>` is the active
        // identity at registry-build time (uuid in prod,
        // `"test"` in fixtures); the description must match by
        // suffix rather than full name so a new user doesn't
        // silently fall through to "(system ability)".
        _ if name.ends_with(".api_key.create") => {
            "Issue a new API key for the calling user. Returns the bearer secret once; \
             the daemon stores only a hashed fingerprint."
        }
        _ if name.ends_with(".api_key.list") => {
            "List the calling user's API keys (fingerprints + metadata, no secrets)."
        }
        _ if name.ends_with(".api_key.revoke") => {
            "Revoke an API key by its fingerprint. The bearer is rejected immediately on \
             every subsequent call."
        }
        _ => "(system ability)",
    }
}

/// JSON Schema for a published system ability's input. Mirrors
/// `description_for` — adding an arm here is the second half of
/// landing a new system ability so it can register against
/// axon-runtime with a real schema (not the empty-object default).
///
/// Unknown names fall back to `{"type":"object"}` — the most
/// permissive shape that still validates as a JSON Schema. A future
/// ability that lands without an arm here is callable but appears
/// as schema-less in MCP `ListTools`; a CI test pins the table
/// against the live registry to surface that drift.
pub fn input_schema_for(name: &str) -> serde_json::Value {
    match name {
        "device.observe.health" => ping::input_schema(),
        "device.observe.network_health" => network_health_ability::input_schema(),
        "device.policy.evaluate" => policy_ability::evaluate_input_schema(),
        "device.policy.simulate" => policy_ability::simulate_input_schema(),
        "device.session.list" => session_ability::list_input_schema(),
        "device.session.attach" => session_ability::attach_input_schema(),
        "device.consent.subscribe" => permission_ability::subscribe_input_schema(),
        "device.consent.decide" => permission_ability::decide_input_schema(),
        "device.consent.list_pending" => permission_ability::list_pending_input_schema(),
        "device.discuss.create" => discuss_ability::create_input_schema(),
        "device.discuss.post" => discuss_ability::post_input_schema(),
        "device.discuss.subscribe" => discuss_ability::subscribe_input_schema(),
        "device.discuss.list_turns" => discuss_ability::list_turns_input_schema(),
        "device.schedule.add" => schedule_ability::add_input_schema(),
        "device.schedule.list" => schedule_ability::list_input_schema(),
        "device.schedule.remove" => schedule_ability::remove_input_schema(),
        "device.schedule.enable" => schedule_ability::enable_input_schema(),
        "device.loop.create" => loop_ability::create_input_schema(),
        "device.loop.status" => loop_ability::status_input_schema(),
        "device.loop.subscribe" => loop_ability::subscribe_input_schema(),
        "device.loop.cancel" => loop_ability::cancel_input_schema(),
        "device.skill.install" => skill_install_ability::install_input_schema(),
        "device.skill.remove" => skill_install_ability::remove_input_schema(),
        "device.skill.upgrade" => skill_install_ability::upgrade_input_schema(),
        "device.mcp.bridge.list_tools" => mcp_bridge_ability::list_tools_input_schema(),
        "device.mcp.bridge.call_tool" => mcp_bridge_ability::call_tool_input_schema(),
        "device.a2a.bridge.list_skills" => a2a_bridge_ability::list_skills_input_schema(),
        "device.a2a.bridge.send_task" => a2a_bridge_ability::send_task_input_schema(),
        "device.a2a.client.send_task" => a2a_client_ability::send_task_input_schema(),
        "device.mcp.client.list" => mcp_client_ability::list_input_schema(),
        "device.mcp.client.call" => mcp_client_ability::call_input_schema(),
        "device.agent.list" => agent_list_ability::list_agents_input_schema(),
        "device.meta.describe" => meta_ability::describe_input_schema(),
        "device.meta.list_abilities" => meta_ability::list_abilities_input_schema(),
        "device.mission.run" => mission_ability::run_input_schema(),
        "device.mission.track" => mission_ability::track_input_schema(),
        "device.mission.cancel" => mission_ability::cancel_input_schema(),
        // AXIOM §"Tier 2.5" Baseline Locomotion — filesystem half.
        "device.fs.read" => fs_ability::input_schema_read(),
        "device.fs.write" => fs_ability::input_schema_write(),
        "device.fs.list" => fs_ability::input_schema_list(),
        "device.fs.edit" => fs_edit_ability::input_schema(),
        "device.process.exec" => process_exec_ability::input_schema(),
        "device.shell.run" => shell_run_ability::input_schema(),
        "device.http.request" => http_request_ability::input_schema(),
        "device.invocation.history.list" => invocation_history_ability::list_history_input_schema(),
        "device.invocation.history.get" => invocation_history_ability::get_history_input_schema(),
        "device.invocation.trace.get" => invocation_history_ability::get_trace_input_schema(),
        "device.terminal.create" => pty_lifecycle_ability::input_schema_create(),
        "device.terminal.close" => pty_lifecycle_ability::input_schema_close(),
        "device.terminal.attach" => pty_attach_ability::input_schema(),
        "device.terminal.input" => pty_io_ability::input_input_schema(),
        "device.terminal.read" => pty_io_ability::read_input_schema(),
        "device.terminal.resize" => pty_io_ability::resize_input_schema(),
        "device.fs.transfer" => file_transfer_ability::input_schema(),
        "device.agent.start" => agent_lifecycle_ability::start_agent_input_schema(),
        "device.agent.stop" => agent_lifecycle_ability::stop_agent_input_schema(),
        "device.node.list" => device_ops_ability::list_nodes_input_schema(),
        "device.node.describe" => device_ops_ability::describe_node_input_schema(),
        "device.node.remove" => device_ops_ability::remove_node_input_schema(),
        "device.ability.deploy" => device_ops_ability::deploy_ability_input_schema(),
        "device.ability.uninstall" => device_ops_ability::uninstall_ability_input_schema(),
        "device.remote.exec" => device_ops_ability::exec_remote_input_schema(),
        "device.node.register" => device_ops_ability::register_self_input_schema(),
        "device.node.deregister" => device_ops_ability::deregister_self_input_schema(),
        "device.mission.discuss_round" => orchestration_ability::discuss_round_input_schema(),
        "device.voice.create_call" => voice_call_ability::create_call_input_schema(),
        "device.voice.show_call" => voice_call_ability::show_call_input_schema(),
        "device.voice.join_call" => voice_call_ability::join_call_input_schema(),
        "device.voice.leave_call" => voice_call_ability::leave_call_input_schema(),
        "device.voice.end_call" => voice_call_ability::end_call_input_schema(),
        "device.voice.watch_call" => voice_call_ability::watch_call_input_schema(),
        "device.voice.report_metrics" => voice_call_ability::report_metrics_input_schema(),
        "device.voice.list_calls" => voice_call_ability::list_calls_input_schema(),
        // RFC-012 §RemoteWebSurface — device.browser.* family.
        "device.browser.open_session" => browser_session_ability::open_session_input_schema(),
        "device.browser.send_input" => browser_session_ability::send_input_input_schema(),
        "device.browser.capture_viewport" => {
            browser_session_ability::capture_viewport_input_schema()
        }
        "device.browser.close_session" => browser_session_ability::close_session_input_schema(),
        "device.admin.status" => admin_status_ability::input_schema(),
        "device.ability.publish" => ability_publish_ability::publish_input_schema(),
        "device.ability.unpublish" => ability_publish_ability::unpublish_input_schema(),
        "device.skill.publish" => skill_publish_ability::publish_input_schema(),
        "device.skill.unpublish" => skill_publish_ability::unpublish_input_schema(),
        "device.skill.list" => skill_publish_ability::list_input_schema(),
        "device.skill.tree" => skill_publish_ability::tree_input_schema(),
        "device.skill.read_file" => skill_publish_ability::read_file_input_schema(),
        "device.skill.write_file" => skill_publish_ability::write_file_input_schema(),
        "device.mission.think" => think_ability::input_schema(),
        // RFC-005 v3.2 A1–A8 — media abilities. Same single-source
        // -of-truth pattern as `description_for` above.
        n if media_abilities::input_schema(n).is_some() => {
            media_abilities::input_schema(n).unwrap()
        }
        list_resources_ability::ABILITY_META_LIST_RESOURCES => {
            list_resources_ability::input_schema()
        }
        // RFC-006-C v0.1 — device-local OpenAI shim. Schemas mirror
        // the OpenAI request envelopes the handler accepts (chat
        // completion body, plus an `auth_token` bearer for the
        // device-local api_key store).
        "device.openai.chat_completions" => serde_json::json!({
            "type": "object",
            "required": ["request"],
            "properties": {
                "request": {
                    "type": "object",
                    "description": "OpenAI-compatible /v1/chat/completions request body."
                },
                "auth_token": {
                    "type": "string",
                    "description": "Bearer token bound to a `<user>.api_key` entry on this host."
                }
            }
        }),
        "device.openai.list_models" => serde_json::json!({
            "type": "object",
            "properties": {
                "auth_token": {
                    "type": "string",
                    "description": "Bearer token bound to a `<user>.api_key` entry on this host."
                }
            }
        }),
        // `<user>.api_key.{create,list,revoke}` — see the matching
        // suffix arms in `description_for` for the rationale on
        // why these match by suffix rather than full name.
        n if n.ends_with(".api_key.create") => serde_json::json!({
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "Optional operator-facing label for the new key."
                }
            }
        }),
        n if n.ends_with(".api_key.list") => serde_json::json!({
            "type": "object",
            "additionalProperties": false
        }),
        n if n.ends_with(".api_key.revoke") => serde_json::json!({
            "type": "object",
            "required": ["fingerprint"],
            "properties": {
                "fingerprint": {
                    "type": "string",
                    "description": "Fingerprint of the key to revoke (from .api_key.list)."
                }
            }
        }),
        _ => serde_json::json!({ "type": "object" }),
    }
}

/// RFC-006 metadata for a published ability. Returns `None` for
/// every existing ability — they emit unchanged TOMLs and on-wire
/// descriptors. PR2 (#196) adds `Some(...)` arms for the eight
/// physical-channel abilities + meta.list_resources, declaring
/// their RFC-006 class (Stream / Query). No Transition consumer
/// exists yet; the renderer + descriptor schema support it but
/// no name returns a Transition variant in v1.
pub fn rfc006_for(name: &str) -> Option<ability_toml::Rfc006Metadata> {
    if let Some(meta) = media_abilities::rfc006(name) {
        return Some(meta);
    }
    if name == list_resources_ability::ABILITY_META_LIST_RESOURCES {
        return Some(list_resources_ability::rfc006());
    }
    None
}

/// Sync bridge so `build_registry_with_services` (sync) can call
/// `reflect_all` (async).
///
/// **Why this is allowed to self-host a runtime — unlike
/// `mcp_executor::block_on_async`.** The two bridges look symmetrical
/// but live on opposite sides of the boot/serve boundary:
///
/// * The daemon's `LocalRpcHandler` runs *inside* the gRPC server's
///   tokio runtime. The MCP executor (`mcp_executor::block_on_async`)
///   therefore MUST find an ambient runtime; the absence of one is an
///   authoring bug and we fail fast.
/// * `build_registry_with_services` runs *before* the gRPC runtime
///   is spawned — it is the daemon's synchronous bootstrap, and is
///   also called from a large body of sync unit tests
///   (`build_registry()` in `real_invoke_tests`, `publish.rs`, etc.).
///   At this call site there is no ambient runtime by design; the
///   `reflect_all` work is a one-shot `tools/list` per upstream, so
///   we mint a single-threaded runtime, drive it to completion, and
///   drop it.
///
/// Returns `Ok(report)` with both successes and per-tool failures
/// (so the caller can log them coherently), or `Err` only when
/// the runtime bridge itself fails — a per-server upstream failure
/// is reported inside the `ReflectResult.failed` field, NOT here.
fn reflect_mcp_upstreams_sync(
    client: &crate::runtime::execution::mcp_client::McpClientService,
    registry: &mut crate::runtime::ability_dispatch::LocalAbilityRegistry,
    owner_agent_ura: &str,
) -> anyhow::Result<crate::runtime::agents::mcp_reflective_registry::ReflectResult> {
    use crate::runtime::agents::mcp_reflective_registry::reflect_all;
    let fut = async move { reflect_all(client, registry, owner_agent_ura).await };
    match tokio::runtime::Handle::try_current() {
        Ok(_handle) => Ok(tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(fut)
        })),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| anyhow::anyhow!("build mcp-reflect runtime: {e}"))?;
            Ok(rt.block_on(fut))
        }
    }
}

/// Sync→async bridge that attaches a `RegistryRefreshSink` to every
/// configured stdio MCP upstream. Called once after `Arc::new(reg)`
/// in `build_registry_with_services`, so each sink can hold a
/// `Weak<LocalAbilityRegistry>` that survives daemon shutdown
/// gracefully.
///
/// Per-server failures are logged and skipped — one broken upstream
/// must not prevent the rest from getting their hot-reload sinks.
/// HTTP transports are silently skipped today: their notification
/// path lives inside SSE responses to RPCs, not in an idle channel,
/// so a "hot-reload sink" on the HTTP side is a separate design.
fn attach_mcp_refresh_sinks_sync(
    client: &std::sync::Arc<crate::runtime::execution::mcp_client::McpClientService>,
    registry: &std::sync::Arc<crate::runtime::ability_dispatch::LocalAbilityRegistry>,
    owner_agent_ura: &str,
    initially_reflected: &std::collections::BTreeMap<String, Vec<String>>,
) {
    use crate::runtime::agents::mcp_reflective_registry::RegistryRefreshSink;
    let svc_for_async = client.clone();
    let registry_weak = std::sync::Arc::downgrade(registry);
    let client_weak = std::sync::Arc::downgrade(client);
    let owner = owner_agent_ura.to_string();
    let reflected_snapshot = initially_reflected.clone();

    let fut = async move {
        let server_names = svc_for_async.server_names().await;
        for name in server_names {
            let Some(spec) = svc_for_async.spec(&name).await else {
                continue;
            };
            if spec.transport != "stdio" {
                // Hot-reload over streamable HTTP is not wired in this
                // pass — the listener model lives on the stdio side
                // only. Silently skip rather than warn so a mixed
                // catalogue doesn't drown the operator in noise.
                continue;
            }
            let reflected_for_server = reflected_snapshot
                .get(&name)
                .cloned()
                .unwrap_or_default();
            let sink = Box::new(RegistryRefreshSink::new(
                registry_weak.clone(),
                client_weak.clone(),
                name.clone(),
                owner.clone(),
                reflected_for_server,
            ));
            if let Err(e) = svc_for_async.register_notification_sink(&name, sink).await {
                let server = name.as_str();
                let err_msg = format!("{e}");
                crate::op_event!(
                    component = mcp_reflective,
                    kind = refresh_sink_attach_failed,
                    level = "warn",
                    server = server,
                    error = err_msg,
                );
            }
        }
    };

    let result = match tokio::runtime::Handle::try_current() {
        Ok(_handle) => {
            tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(fut));
            Ok(())
        }
        Err(_) => match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => {
                rt.block_on(fut);
                Ok(())
            }
            Err(e) => Err(format!("build mcp-refresh-sink runtime: {e}")),
        },
    };
    if let Err(e) = result {
        crate::op_event!(
            component = mcp_reflective,
            kind = hot_reload_sink_skipped,
            level = "warn",
            error = e,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Semantic layer for an ability. See
    /// docs/rfc/AXON-RFC-001-ability-layers.md for the contract each
    /// layer enforces. The classifier below + the
    /// `ability_layer_classification_is_complete` test together
    /// guarantee every published name lands in exactly one layer.
    #[derive(Debug, PartialEq, Eq)]
    enum AbilityLayer {
        /// Pure, side-effect free, deterministic for a catalog snapshot.
        Introspection,
        /// Pure decision functions (no mutation of catalog state).
        /// `consent.decide` is the documented exception: write-only-
        /// after-decision.
        Control,
        /// Derived state only; never triggers behaviour elsewhere.
        Observation,
        /// Per-feature business verbs (chat, schedule, loop, discuss,
        /// session, skill management). Not subject to the
        /// layer-purity rules; they ARE the work.
        Operational,
    }

    /// Classify a published ability name by the §"three layers"
    /// model. A name with no match returns `None` and the
    /// completeness test below fails — forcing the author of any
    /// new ability to either pick a layer or update this table.
    fn classify_ability(name: &str) -> Option<AbilityLayer> {
        // Per-agent chat handlers are operational by definition.
        if name.ends_with(".chat") {
            return Some(AbilityLayer::Operational);
        }
        match name {
            // ── Introspection ───────────────────────────────────
            "device.meta.describe"
            | "device.meta.list_abilities"
            // `mission.track` reads the persisted run dir of a
            // prior mission.run. Pure read of derived state →
            // Introspection, same logic that puts schedule.list
            // / loop.status here.
            | "device.mission.track"
            | "device.mcp.bridge.list_tools"
            // mcp.client.list — aggregate read of every configured
            // upstream MCP server's tools/list. No mutation;
            // belongs with the introspection-layer reads.
            | "device.mcp.client.list"
            | "device.a2a.bridge.list_skills"
            | "device.agent.list"
            | "device.invocation.history.list"
            | "device.invocation.history.get"
            | "device.invocation.trace.get"
            | "device.session.list"
            | "device.consent.list_pending"
            // RFC-005 v3.2 A9 — meta.list_resources is a pure read of
            // the local resources table (same shape as
            // meta.list_abilities); Introspection by definition.
            | "device.meta.list_resources"
            // discuss.list_turns — RPC snapshot of a room transcript.
            // Pure read; same Introspection class as schedule.list.
            | "device.discuss.list_turns"
            | "device.schedule.list"
            | "device.loop.status"
            // skill.list / tree / read_file — private skill package
            // inventory and source inspection. Pure reads.
            | "device.skill.list"
            | "device.skill.tree"
            | "device.skill.read_file" => Some(AbilityLayer::Introspection),
            // ── Control / decision ──────────────────────────────
            "device.policy.evaluate"
            | "device.policy.simulate"
            | "device.consent.decide"
            | "device.consent.subscribe" => Some(AbilityLayer::Control),
            // ── Observation ─────────────────────────────────────
            "device.observe.health"
            | "device.observe.network_health"
            | "device.admin.status" => Some(AbilityLayer::Observation),
            // ── Operational (per-feature business verbs) ────────
            "device.session.attach"
            | "device.agent.start"
            | "device.agent.stop"
            | "device.skill.install"
            | "device.skill.remove"
            | "device.skill.upgrade"
            // device-hosted node/ability/remote operations. list_nodes /
            // describe_node read state but conceptually they sit
            // with the federation-tier *operations* (peer
            // enumeration, network health) — Operational by
            // intent, mirroring how schedule.list / loop.status
            // got bumped into the introspection layer because they
            // describe daemon-managed state. The remaining
            // verbs (remove_node, deploy_ability, uninstall_ability,
            // exec_remote, register_self, deregister_self)
            // mutate state — Operational unambiguous.
            | "device.node.list"
            | "device.node.describe"
            | "device.node.remove"
            | "device.ability.deploy"
            | "device.ability.uninstall"
            | "device.remote.exec"
            | "device.node.register"
            | "device.node.deregister"
            // device.terminal.* shell-session lifecycle abilities.
            // create / close mutate session state; input / read /
            // resize push or pull data over an established session;
            // attach binds the bidi data plane. All operational
            // because each call IS the work for that session step.
            | "device.terminal.attach"
            | "device.terminal.create"
            | "device.terminal.close"
            | "device.terminal.input"
            | "device.terminal.read"
            | "device.terminal.resize"
            // mission.discuss_round — sub-turn orchestration
            // ability. Same Operational class as easynet.run /
            // mission.run because the ability IS the work
            // (running one human-bracketed sub-turn of a
            // multi-agent discussion).
            | "device.mission.discuss_round"
            // mission.think — long-running worker+judge loop. Same
            // Operational rationale: the ability IS the work
            // (running an N-cycle reflective loop with two
            // independent chat sessions).
            | "device.mission.think"
            // voice.* call signaling abilities. State-mutating
            // (create / join / leave / end / report_metrics) and
            // state-reading (show / watch) — Operational by intent
            // because the call IS the work. Same shape as
            // discuss.subscribe / loop.subscribe sit here.
            | "device.voice.create_call"
            | "device.voice.show_call"
            | "device.voice.join_call"
            | "device.voice.leave_call"
            | "device.voice.end_call"
            | "device.voice.watch_call"
            | "device.voice.report_metrics"
            | "device.voice.list_calls"
            // mcp.bridge.call_tool / a2a.bridge.send_task — both
            // dispatch into another local ability; the side effects
            // come from that dispatch, not the bridge itself. Sit
            // with the operational verbs because the call surface
            // IS the work.
            | "device.mcp.bridge.call_tool"
            // mcp.client.call — outbound mirror of bridge.call_tool.
            // Same operational classification: dispatching
            // delegates side effects to the upstream tool.
            | "device.mcp.client.call"
            | "device.a2a.bridge.send_task"
            // a2a.client.send_task — outbound mirror of bridge.send_task.
            // Same operational classification: dispatching crosses
            // a wire and mutates the remote node's state.
            | "device.a2a.client.send_task"
            | "device.discuss.create"
            | "device.discuss.post"
            | "device.discuss.subscribe"
            | "device.schedule.add"
            | "device.schedule.remove"
            | "device.schedule.enable"
            | "device.loop.create"
            | "device.loop.subscribe"
            | "device.loop.cancel"
            // EAL orchestration. easynet.run / mission.run compile
            // and execute a program (potentially multi-step,
            // potentially cross-agent); easynet.cancel mutates the
            // run state of an in-flight mission. Same Operational
            // class as loop.{create,cancel} for the same reason —
            // the ability IS the work.
            | "device.mission.run"
            | "device.mission.cancel"
            // ability.publish / ability.unpublish / skill.publish /
            // skill.unpublish — curator-driven sinks for judge-validated
            // experience. State-mutating (writes/removes manifests under
            // an agent's workspace). Operational because the ability IS
            // the work, in the same class as device.ability.deploy /
            // device.skill.install.
            | "device.ability.publish"
            | "device.ability.unpublish"
            | "device.skill.publish"
            | "device.skill.unpublish"
            | "device.skill.write_file"
            // AXIOM §"Tier 2.5" Baseline Locomotion Profile,
            // filesystem half. fs.read is technically read-only
            // but it returns business content, not just metadata
            // — Operational rather than Observation. fs.write
            // mutates state. fs.list returns directory metadata
            // but its purpose is to enable subsequent fs.read /
            // fs.write — Operational by intent.
            | "device.fs.read"
            | "device.fs.write"
            | "device.fs.list"
            | "device.fs.edit"
            // AXIOM Tier 2.5 execution members. process.exec
            // and shell.run are unconditionally Operational —
            // they spawn processes that may do anything; even
            // with the 8-stage shellguard pipeline gating
            // shell.run dispatch, the layer classification
            // tracks privilege not invocation safety.
            | "device.process.exec"
            | "device.shell.run"
            | "device.http.request"
            | "device.fs.transfer"
            // RFC-005 v3.2 A1–A8 — physical-channel media verbs.
            // Operational by intent: each one drives an external
            // device (mic / camera / speaker / screen) or remote
            // model (voice / asr). Subject = resource_ura.
            | "device.mic.subscribe"
            | "device.camera.subscribe"
            | "device.camera.snapshot"
            | "device.screen.subscribe"
            | "device.screen.snapshot"
            | "device.speaker.publish"
            | "device.voice.subscribe"
            | "device.voice.transcribe"
            // RFC-006-C v0.1 — device-local OpenAI protocol shim.
            // chat_completions IS the work (forwards a generation
            // request to a host-local chat-base ability);
            // list_models reads the caller's dispatch-grant set,
            // but its operational role is "answer /v1/models for
            // the OpenAI surface" — both are Operational rather
            // than Introspection.
            | "device.openai.chat_completions"
            | "device.openai.list_models"
            // RFC-012 §RemoteWebSurface — device.browser.* family.
            // Operational by intent: opening a WebView session,
            // streaming frames, injecting input, closing the
            // session all drive an external surface (the user's
            // system WebView) under the caller's identity. Same
            // class as media/* verbs.
            | "device.browser.open_session"
            | "device.browser.send_input"
            | "device.browser.capture_viewport"
            | "device.browser.close_session"
            => Some(AbilityLayer::Operational),
            // `<user>.api_key.{create,list,revoke}` — user-rooted
            // credential-lifecycle verbs. `<user>` is the active
            // identity (uuid in prod, `"test"` in fixtures), so we
            // match by suffix rather than enumerating one identity.
            // All three are Operational because the ability IS the
            // work (issuing / listing / revoking a credential), in
            // the same class as ability.publish / skill.publish.
            n if n.ends_with(".api_key.create")
                || n.ends_with(".api_key.list")
                || n.ends_with(".api_key.revoke") =>
            {
                Some(AbilityLayer::Operational)
            }
            _ => None,
        }
    }

    #[test]
    fn ability_layer_classification_is_complete() {
        // The audit story (RFC docs/AXON-RFC-001-ability-layers.md)
        // says every published ability MUST belong to exactly one
        // semantic layer. A new ability that lands without a
        // classify_ability arm trips this test, forcing the author
        // to either pick a layer or amend the layer doc.
        let names = published_ability_names();
        let unclassified: Vec<String> = names
            .iter()
            // device.keyring.* abilities have their own ontology
            // (RFC-002 §3.3) and are not classified by the system
            // ability layer table.
            .filter(|n| !n.starts_with("device.keyring."))
            .filter(|n| classify_ability(n).is_none())
            .cloned()
            .collect();
        assert!(
            unclassified.is_empty(),
            "abilities missing a layer classification: {unclassified:?}\n\
             Add an arm to classify_ability() in src/runtime/agents/mod.rs \
             and update docs/rfc/AXON-RFC-001-ability-layers.md."
        );
    }

    #[test]
    fn introspection_layer_includes_all_three_discovery_planes() {
        // The discovery-planes invariant from
        // docs/rfc/AXON-RFC-001-discovery-planes.md: meta.list_abilities,
        // mcp.bridge.list_tools, and a2a.bridge.list_skills MUST all
        // classify as Introspection. A regression that moved one of
        // them to a different layer would fragment the discovery story.
        for name in [
            "device.meta.list_abilities",
            "device.mcp.bridge.list_tools",
            "device.a2a.bridge.list_skills",
        ] {
            assert_eq!(
                classify_ability(name),
                Some(AbilityLayer::Introspection),
                "{name} must classify as Introspection (discovery plane)"
            );
        }
    }

    #[test]
    fn build_registry_is_non_empty_and_includes_ping() {
        // Every v1 daemon publishes at least `device.observe.health` so a
        // peer wanting to test reachability has a known ability.
        // A regression that emptied this list would silently break
        // discovery + smoke tests.
        let reg = build_registry();
        let names = reg.list_abilities();
        assert!(
            names.iter().any(|n| n == "device.observe.health"),
            "device.observe.health must be in the v1 registry; got {names:?}"
        );
    }

    #[test]
    fn published_ability_names_matches_live_registry() {
        // The label-publishing helper and the dispatch registry
        // must agree byte-for-byte. A regression that returned a
        // hard-coded list would let the publisher advertise
        // abilities the dispatcher cannot route.
        let live = build_registry().list_abilities();
        let advertised = published_ability_names();
        assert_eq!(live, advertised);
    }

    #[test]
    fn every_published_ability_has_a_toml_byte_for_byte_matching_the_renderer() {
        // The TOML descriptors in abilities/system/ are the
        // source of truth for external discovery tools. They are
        // GENERATED from `render_ability_toml(name,
        // description_for(name), input_schema_for(name))`. This
        // test enforces that the on-disk file is byte-for-byte
        // identical to what the renderer produces; if the
        // dispatcher's metadata changed and a maintainer forgot
        // to regenerate, this test names every drifted ability
        // and tells them how to fix it.
        let mut missing: Vec<String> = Vec::new();
        let mut drift: Vec<String> = Vec::new();
        for meta in published_abilities() {
            let toml_path = format!("abilities/system/{}.ability.toml", meta.name);
            let on_disk = match std::fs::read_to_string(&toml_path) {
                Ok(body) => body,
                Err(_) => {
                    missing.push(meta.name.clone());
                    continue;
                }
            };
            let _ = rfc006_for(&meta.name);
            let expected =
                ability_toml::render_ability_toml(&meta.name, meta.description, &meta.input_schema);
            if on_disk != expected {
                drift.push(meta.name.clone());
            }
        }
        let mut errors: Vec<String> = Vec::new();
        if !missing.is_empty() {
            errors.push(format!(
                "no TOML on disk for: {missing:?}\n\
                 -> run `cargo run --bin gen-ability-tomls` to create them"
            ));
        }
        if !drift.is_empty() {
            errors.push(format!(
                "TOML on disk differs from renderer output for: {drift:?}\n\
                 -> run `cargo run --bin gen-ability-tomls` to regenerate"
            ));
        }
        assert!(
            errors.is_empty(),
            "abilities/system TOML descriptor drift:\n  {}",
            errors.join("\n  ")
        );
    }

    /// Walk every published ability and confirm a handler is
    /// registered under SOME invocation mode (RPC, Stream, or
    /// Bidi). Distinguishes "ability advertised in
    /// list_abilities() but dispatcher returns ABILITY_NOT_FOUND"
    /// from "ability is callable". This is the bare minimum for
    /// the question "is this ability really wired".
    ///
    /// What this DOES NOT verify:
    ///   * the handler implementation is correct (most need
    ///     valid args, services, real I/O — those have their
    ///     own per-module tests),
    ///   * the response shape matches the documented schema,
    ///   * end-to-end behavior over the wire.
    /// What it DOES verify:
    ///   * `register(...)` was called for every published name
    ///     (catches the slice-16 bug class: file present, never
    ///     wired into build_registry_with_services),
    ///   * the registration mode matches the dispatcher's
    ///     expectation (catches "registered as Stream but
    ///     get_rpc() returns None" type mismatches).
    #[test]
    fn every_published_ability_resolves_to_a_handler() {
        let reg = build_registry();
        let names: Vec<String> = reg.list_abilities();
        let mut unresolved: Vec<String> = Vec::new();
        for name in &names {
            // <agent>.chat handlers register as Stream. Most
            // system abilities register as RPC. Bidi is rare
            // (PTY attach). We accept any of the three.
            let has_rpc = reg.get_rpc(name).is_some();
            let has_stream = reg.get_stream(name).is_some();
            let has_bidi = reg.get_bidi(name).is_some();
            if !(has_rpc || has_stream || has_bidi) {
                unresolved.push(name.clone());
            }
        }
        assert!(
            unresolved.is_empty(),
            "abilities listed by list_abilities() but with NO handler registered: {unresolved:?}"
        );
    }

    /// For every RPC-mode ability with no-arg or empty-args
    /// schema, actually invoke it through the dispatcher with
    /// `{}` and confirm the call returns SOME result (Ok(value)
    /// or a structured Err). The point is: we exercise the full
    /// dispatch path for every ability we can — registry lookup,
    /// handler invocation, response materialisation — not just
    /// "the function pointer exists".
    ///
    /// Rationale for the empty-args scope:
    ///   * Many RPC abilities have required fields (e.g.
    ///     fs.read needs `path`). Calling them with `{}` will
    ///     return Err(`missing required field`) which is the
    ///     CORRECT response and proves the handler runs end-to-
    ///     end. We accept Err here as PASS — the handler
    ///     reached arg-validation.
    ///   * What we reject is: panic (test framework catches),
    ///     ABILITY_NOT_FOUND error (means dispatch never reached
    ///     a handler), or a hang (test would time out).
    #[test]
    fn every_rpc_ability_actually_dispatches_through_to_its_handler() {
        use crate::runtime::ability_dispatch::AbilityDispatcher;
        use crate::runtime::gateway::NoopGateway;
        use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

        let reg = build_registry();
        let dispatcher = AbilityDispatcher::new(Arc::clone(&reg), Arc::new(NoopGateway::new()));
        let names = reg.list_abilities();

        let mut invoked_ok: Vec<String> = Vec::new();
        let mut invoked_err: Vec<(String, String)> = Vec::new();
        let mut not_found: Vec<String> = Vec::new();
        let mut not_rpc: Vec<String> = Vec::new();

        for name in &names {
            // Only invoke things that ARE RPC. Stream / Bidi
            // abilities skip this stage; the previous test
            // confirmed they have a handler under their mode.
            if reg.get_rpc(name).is_none() {
                not_rpc.push(name.clone());
                continue;
            }
            let target = InvocationTarget {
                scope: TargetScope::Local,
                ability: name.clone(),
                normalized_args: serde_json::json!({}),
                call_mode: CallMode::Rpc,
                subject: None,
            };
            match dispatcher.execute_rpc(target) {
                Ok(_) => invoked_ok.push(name.clone()),
                Err(e) => {
                    let msg = format!("{e}");
                    if msg.to_ascii_lowercase().contains("no rpc handler")
                        || msg.contains("ABILITY_NOT_FOUND")
                    {
                        not_found.push(name.clone());
                    } else {
                        invoked_err.push((name.clone(), msg));
                    }
                }
            }
        }

        // Test-only diagnostic summary so a green run still shows
        // what was actually exercised (visible with
        // `cargo test ... -- --nocapture`). These are deliberately
        // raw `eprintln!` and NOT `op_event!`: they are test-binary
        // human-readable output, not daemon operator events, so
        // the `[component] kind=event` schema would only add
        // unrelated noise to a developer's terminal. The `op_event!`
        // discipline applies to `src/runtime/**` daemon code paths,
        // not to the test bodies that exercise them.
        eprintln!(
            "ability invoke smoke: {} OK, {} errored-but-reached-handler, {} skipped (non-RPC)",
            invoked_ok.len(),
            invoked_err.len(),
            not_rpc.len()
        );
        if !invoked_err.is_empty() {
            eprintln!("  errored (handler reached, normal):");
            for (n, m) in &invoked_err {
                let preview: String = m.chars().take(80).collect();
                eprintln!("    {n}: {preview}");
            }
        }
        if !not_rpc.is_empty() {
            eprintln!("  skipped (registered as Stream/Bidi): {not_rpc:?}");
        }

        assert!(
            not_found.is_empty(),
            "abilities advertised but dispatcher could not find an RPC handler: {not_found:?}"
        );
    }

    #[test]
    fn build_registry_actually_contains_every_baseline_locomotion_ability() {
        // Pin the AXIOM Tier 2.5 surface: every member of the
        // Baseline Locomotion Profile MUST be registered in the
        // live registry. A regression that adds a `pub mod` but
        // forgets the `register(&mut reg)` call would leave the
        // ability invisible to the dispatcher even though the
        // module compiles. This test catches that.
        let reg = build_registry();
        let names: std::collections::BTreeSet<String> = reg.list_abilities().into_iter().collect();
        let must_have = [
            // Filesystem half
            "device.fs.read",
            "device.fs.write",
            "device.fs.list",
            "device.fs.edit",
            // Execution half
            "device.process.exec",
            "device.shell.run",
            // Outbound network
            "device.http.request",
            // Interactive PTY trio
            "device.terminal.create",
            "device.terminal.close",
            "device.terminal.attach",
            // Operator surface added in slice 16
            "device.admin.status",
            "device.agent.start",
            "device.agent.stop",
        ];
        let missing: Vec<&str> = must_have
            .iter()
            .filter(|n| !names.contains(**n))
            .copied()
            .collect();
        assert!(
            missing.is_empty(),
            "Baseline Locomotion abilities NOT registered: {missing:?}.\n\
             Live registry has {} abilities: {:?}",
            names.len(),
            names
        );
    }

    #[test]
    fn published_abilities_includes_skill_list_with_real_metadata() {
        // Load-bearing for the EasyNet frontend's Skills page: the
        // backend invokes `device.skill.list` against the target node.
        // A regression that dropped it from `published_abilities()`
        // would silently empty the Skills page across the device set.
        let metas = published_abilities();
        let skill = metas
            .iter()
            .find(|m| m.name == "device.skill.list")
            .expect("device.skill.list must be in published_abilities");
        // Description must NOT be the unknown-name fallback.
        // `(system ability)` is what `description_for` returns when
        // an ability is added without an arm here; pin against it so
        // a future ability that lands without metadata trips the
        // test instead of shipping a generic blurb to the frontend.
        assert_ne!(
            skill.description, "(system ability)",
            "device.skill.list must have a real description, not the fallback"
        );
        // Input schema must be a JSON Schema object (the wire shape
        // axon-runtime stores). Empty `{}` would also pass `is_object`,
        // so additionally pin the `type` field.
        assert_eq!(
            skill.input_schema.get("type").and_then(|v| v.as_str()),
            Some("object"),
            "input schema must declare type:object; got {:?}",
            skill.input_schema
        );
        assert!(
            !skill.hints.streaming_only && !skill.hints.bidi_only,
            "device.skill.list must stay unary-only; got hints {:?}",
            skill.hints
        );
    }

    #[test]
    fn published_abilities_marks_server_stream_routes_as_streaming_only() {
        let metas = published_abilities();
        for name in [
            "device.consent.subscribe",
            "device.discuss.subscribe",
            "device.loop.subscribe",
            "device.session.attach",
            "device.mic.subscribe",
            "device.camera.subscribe",
            "device.screen.subscribe",
            "device.voice.subscribe",
        ] {
            let meta = metas
                .iter()
                .find(|m| m.name == name)
                .unwrap_or_else(|| panic!("{name} must be published"));
            assert!(
                meta.hints.streaming_only,
                "{name} must advertise streaming_only so callers use InvokeStream"
            );
            assert!(!meta.hints.bidi_only, "{name} is server-stream, not bidi");
        }
    }

    #[test]
    fn published_abilities_marks_bidi_routes_as_bidi_only() {
        let metas = published_abilities();
        for name in [
            "device.fs.transfer",
            "device.terminal.attach",
            "device.speaker.publish",
            "device.voice.transcribe",
        ] {
            let meta = metas
                .iter()
                .find(|m| m.name == name)
                .unwrap_or_else(|| panic!("{name} must be published"));
            assert!(meta.hints.bidi_only, "{name} must advertise bidi_only");
            assert!(
                !meta.hints.streaming_only,
                "{name} must not masquerade as server-stream"
            );
        }
    }

    #[test]
    fn discovery_hints_leave_agent_chat_on_unary_control_plane_path() {
        use crate::registry::agents::{AgentEntry, AgentType};
        let mut agents = AgentRegistry::default();
        agents
            .agents
            .insert("alice".into(), AgentEntry::new(AgentType::ClaudeCode, None));
        let reg = build_registry_with_services(
            Arc::new(SessionService::new()),
            Arc::new(PermissionService::new()),
            Arc::new(DiscussService::new()),
            Arc::new(ScheduleService::new()),
            Arc::new(LoopService::new()),
            None,
            &agents,
            Arc::new(Vec::new()),
            crate::runtime::agents::PagesIdentity::default(),
        );
        let hints = discovery_hints_for(&reg, "alice.chat");
        assert!(
            !hints.streaming_only && !hints.bidi_only,
            "alice.chat must stay on the unary/OpenAI control-plane path until generic InvokeStream support lands; got {:?}",
            hints
        );
    }

    #[test]
    fn published_abilities_excludes_per_agent_chat_handlers() {
        // `<agent>.chat` is published via the per-agent manifest path
        // (`runtime::publish::republish_abilities_via_advertise`) off the
        // on-disk `chat.ability.toml`. Re-publishing it through the
        // system path would double-register with a synthesised schema
        // that shadows the manifest's real one. The filter in
        // `published_abilities()` enforces this; pin it.
        use crate::registry::agents::{AgentEntry, AgentType};
        let mut agents = AgentRegistry::default();
        agents
            .agents
            .insert("alice".into(), AgentEntry::new(AgentType::ClaudeCode, None));
        let reg = build_registry_with_services(
            Arc::new(SessionService::new()),
            Arc::new(PermissionService::new()),
            Arc::new(DiscussService::new()),
            Arc::new(ScheduleService::new()),
            Arc::new(LoopService::new()),
            None,
            &agents,
            Arc::new(Vec::new()),
            crate::runtime::agents::PagesIdentity::default(),
        );
        // Sanity: the registry itself does include alice.chat.
        assert!(reg.list_abilities().iter().any(|n| n == "alice.chat"));
        // But the system publisher's view excludes it. We can't call
        // published_abilities() with this custom registry directly
        // (it goes through build_registry()), so instead assert the
        // filter property: every entry's name does NOT end with .chat.
        for meta in published_abilities() {
            assert!(
                !meta.name.ends_with(".chat"),
                "published_abilities must filter out *.chat (came in via per-agent manifest); \
                 found {} which would double-register",
                meta.name
            );
        }
    }

    #[test]
    fn description_for_and_input_schema_for_cover_every_published_name() {
        // Adding a new ability to build_registry without also adding
        // arms to `description_for`/`input_schema_for` would let it
        // ship with the unknown-name fallback ("(system ability)" and
        // empty `{type: object}` schema). Pin the contract that every
        // published name has real metadata.
        for name in published_ability_names() {
            // `<agent>.chat` is the documented exception — its
            // description lives in the manifest, not the table — so
            // skip it here. (The `published_abilities` filter already
            // strips it from the publisher's view.)
            if name.ends_with(".chat") {
                continue;
            }
            // `device.keyring.*` abilities are RFC-002-owner-scoped;
            // their metadata lives in `keyring::abilities`, not the
            // system descriptor table. Same exception shape as chat.
            if name.starts_with("device.keyring.") {
                continue;
            }
            let desc = description_for(&name);
            assert_ne!(
                desc, "(system ability)",
                "{name} is missing a description_for arm — add one in runtime::system::mod"
            );
            let schema = input_schema_for(&name);
            // The default fallback returns `{"type":"object"}` with
            // NO other keys. A real arm always pins something more —
            // `properties`, `additionalProperties`, `oneOf`, etc. —
            // even for genuinely-no-arg abilities (e.g.
            // `consent.subscribe` declares
            // `additionalProperties: false`). Distinguishing the
            // fallback from an authored "no-arg" schema by structure
            // (does the object have any key besides `type`?) is
            // strictly stronger than a name allowlist.
            let obj = schema
                .as_object()
                .unwrap_or_else(|| panic!("{name} schema must be a JSON object"));
            let has_only_type = obj.len() == 1 && obj.contains_key("type");
            assert!(
                !has_only_type,
                "{name} fell through to the default `{{type: object}}` schema; \
                 add an input_schema_for arm (declare additionalProperties: false \
                 even if the ability is genuinely no-arg)"
            );
        }
    }

    #[test]
    fn registry_includes_chat_handler_per_registered_agent() {
        // After Phase 3 wired chat as a system ability, every agent
        // in the registry should produce a `<agent>.chat` handler in
        // the unified LocalAbilityRegistry. This is the load-bearing
        // property that lets the proxy dispatch chat through the
        // same registry as ping/session/permission.
        use crate::registry::agents::{AgentEntry, AgentType};
        let mut agents = AgentRegistry::default();
        agents
            .agents
            .insert("alice".into(), AgentEntry::new(AgentType::ClaudeCode, None));
        agents
            .agents
            .insert("bob".into(), AgentEntry::new(AgentType::Codex, None));
        let reg = build_registry_with_services(
            Arc::new(SessionService::new()),
            Arc::new(PermissionService::new()),
            Arc::new(DiscussService::new()),
            Arc::new(ScheduleService::new()),
            Arc::new(LoopService::new()),
            None,
            &agents,
            Arc::new(Vec::new()),
            crate::runtime::agents::PagesIdentity::default(),
        );
        let names = reg.list_abilities();
        assert!(
            names.iter().any(|n| n == "alice.chat"),
            "alice.chat must be registered; got {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "bob.chat"),
            "bob.chat must be registered; got {names:?}"
        );
    }

    #[test]
    fn build_registry_registers_keyring_abilities_when_not_disabled() {
        // Run in a child-process-style isolation: redirect the
        // keyring file path to a tempdir + clear DISABLE so the
        // auto-init path runs. The default tests set DISABLE.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("keyring.json");
        let prev_disable = std::env::var_os("EASYNET_KEYRING_DISABLE");
        let prev_path = std::env::var_os("EASYNET_KEYRING_PATH");
        let prev_pass = std::env::var_os("EASYNET_KEYRING_PASS");
        std::env::remove_var("EASYNET_KEYRING_DISABLE");
        std::env::set_var("EASYNET_KEYRING_PATH", &path);
        std::env::set_var("EASYNET_KEYRING_PASS", "test-pass-keyring-init");

        let agents = AgentRegistry::default();
        let reg = build_registry_with_services(
            Arc::new(SessionService::new()),
            Arc::new(PermissionService::new()),
            Arc::new(DiscussService::new()),
            Arc::new(ScheduleService::new()),
            Arc::new(LoopService::new()),
            None,
            &agents,
            Arc::new(Vec::new()),
            crate::runtime::agents::PagesIdentity::default(),
        );
        let names = reg.list_abilities();

        // Restore env before assertions so a panic doesn't leak
        // environment changes into other tests in the same binary.
        match prev_disable {
            Some(v) => std::env::set_var("EASYNET_KEYRING_DISABLE", v),
            None => std::env::remove_var("EASYNET_KEYRING_DISABLE"),
        }
        match prev_path {
            Some(v) => std::env::set_var("EASYNET_KEYRING_PATH", v),
            None => std::env::remove_var("EASYNET_KEYRING_PATH"),
        }
        match prev_pass {
            Some(v) => std::env::set_var("EASYNET_KEYRING_PASS", v),
            None => std::env::remove_var("EASYNET_KEYRING_PASS"),
        }

        // All 10 abilities must be present under device.keyring.*.
        for verb in [
            "create",
            "list",
            "get_public",
            "sign",
            "rotate",
            "revoke",
            "expire_set",
            "bind_subject",
            "peer_add",
            "peer_list",
        ] {
            let want = format!("device.keyring.{verb}");
            assert!(
                names.iter().any(|n| n == &want),
                "{want} must be registered; got {names:?}"
            );
        }
        assert!(path.exists(), "keyring file must have been auto-created");
    }

    /// **M5 lint** — every catalogue entry must be partitioned
    /// under one of the four owner-prefix shapes from the
    /// truth-table spec: `device.*`, `hub.*`, `<agent-id>.*`,
    /// `<user-id>.*`. A new system ability that lands without
    /// a `device.` / `hub.` prefix fails this test, forcing the
    /// author to either (a) fix the registration site or (b)
    /// argue with the spec. The closed sets of legacy roots
    /// (`fs.`, `01HUB.`, …) MUST stay drained — M3
    /// already deleted the registrations, M5 pins them.
    #[test]
    fn published_catalogue_uses_only_canonical_prefixes() {
        const FORBIDDEN_LEGACY_HEADS: &[&str] = &[
            "fs", "http", "shell", "process", "observe", "admin", "easynet", "meta", "mission",
            "schedule", "loop", "discuss", "mcp", "a2a", "policy", "ability", "camera", "mic",
            "screen", "speaker", "voice", "skill", "consent", "01HUB",
        ];
        let names = published_ability_names();
        let mut violations: Vec<String> = Vec::new();
        for n in &names {
            if let Some((head, _)) = n.split_once('.') {
                if FORBIDDEN_LEGACY_HEADS.contains(&head) {
                    violations.push(n.clone());
                }
            }
        }
        assert!(
            violations.is_empty(),
            "post-M3 catalogue must use canonical prefixes only; \
             found legacy-prefixed names: {violations:?}"
        );
    }

    /// **M5 lint** — `<self>` token never appears as a first
    /// segment in the published catalogue. The wire-pinned trio
    /// (`<self>.session`, `<self>.invoke_remote`,
    /// `<self>.register_device_pubkey`) goes through wire-only
    /// constants; they are NOT registered into the discoverable
    /// catalogue. If they ever leak, this test fails and the
    /// regression is caught at CI rather than in an LLM seeing
    /// a `<self>.*` entry and getting confused.
    #[test]
    fn published_catalogue_never_contains_self_alias() {
        let names = published_ability_names();
        let leaks: Vec<&String> = names.iter().filter(|n| n.starts_with("<self>")).collect();
        assert!(
            leaks.is_empty(),
            "post-M5 catalogue must not expose <self>.* names; got {leaks:?}"
        );
    }
}
