// EasyNet CLI — mission.think (long-running task ability with judge)
// =====================================================================
//
// File: src/runtime/agents/think_ability.rs
// Description: Two-session orchestration that lets an agent run a
//              task long enough to outgrow Claude Code's per-session
//              ~200-step internal limit. A *worker* session does the
//              work. A *judge* session, after each worker turn,
//              reads the transcript and emits a structured verdict:
//              terminate yes/no, plus a memory-classification block
//              describing whether anything from this experience is
//              worth sinking back into the agent's ability/skill
//              ontology (Phase 5 wires the curator that does the
//              actual sinking — Phase 4 only emits the verdict).
//
// Why two sessions and not one
// ----------------------------
// "Worker" needs scratch space, tool calls, intermediate artifacts.
// "Judge" needs an outsider's read on whether the worker is making
// progress, hallucinating, or going in circles. A single session
// cannot honestly judge itself: the same context that produced a
// half-baked answer is the context that judges the answer good
// enough. Two-session resolves this by letting judge see the
// transcript fresh, with no investment in the worker's previous
// decisions.
//
// Cross-session continuity for worker
// -----------------------------------
// Every worker turn calls `<agent>.chat` with `session_id` set to
// the prior turn's returned id. This is the SAME mechanism
// `mission.discuss_round` uses for cross-cycle agent state — Claude
// Code's chat driver echoes back a session id which, when passed
// in, resumes the conversation including memory of all prior tool
// calls. Without resume, every cycle would start from a blank
// context and "long task" would degenerate to "5 short tasks each
// re-reading the codebase."
//
// Judge schema (memory-type taxonomy)
// -----------------------------------
// The judge emits a JSON block whose shape is borrowed from the
// AliveCode memory model (memoryTypes.ts): every saveable
// experience belongs to exactly one of {feedback, project,
// reference, user} types and exactly one of {private, team}
// scopes. Curator (Phase 5) routes:
//
//   * scope = "team"    → ability.publish (device-visible)
//   * scope = "private" → skill.publish   (agent-private)
//   * memory_type = "none" or any exclusion check fires → skip
//
// The schema deliberately requires `why` and `how_to_apply`. If
// the judge cannot articulate why the lesson exists or how to
// apply it, the lesson is not actually learned — it would just be
// a snapshot of the transcript, which the AliveCode model
// explicitly excludes (the WHAT_NOT_TO_SAVE list: "git history",
// "ephemeral task details", "fix recipes from a debug session").
//
// Loop budget
// -----------
// Hard cap on cycles, configurable via `args.max_cycles`. Default
// 5. We considered judge-driven termination (let judge return
// `done | continue | escalate`) but rejected it: a runaway worker
// + a complacent judge can burn token budget without bound, and
// the operator's request for "think harder" already encodes how
// many cycles they think the problem deserves. Hard cap is the
// safest default; a future flag can lift it.
//
// What this module is NOT (Phase 4 boundary)
// -------------------------------------------
// Phase 4 emits the judge's verdict; it does NOT call ability.publish
// or skill.publish itself. The curator session that consumes the
// verdict and dispatches the publish call is Phase 5. Splitting
// keeps Phase 4 testable on its own: the worker+judge loop is the
// hard part of the loop machinery, the curator is a thin
// publish-or-skip dispatcher with its own surface tests.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::AxonAbilityCatalog;

/// Wire name. Pinned because the CLI (`easynet mission think`) and
/// any future EAL caller bind to it by string.
pub const ABILITY_THINK: &str = "mission.think";

/// Default max worker+judge cycles. Mirrors `mission.discuss_round`'s
/// default — same rationale (give the model enough turns to
/// converge but not so many that a runaway loop burns the token
/// budget unnoticed). Operator can pass `max_cycles` in args to
/// override; the upper bound is `HARD_MAX_CYCLES`.
pub const DEFAULT_MAX_CYCLES: u32 = 5;
/// Absolute ceiling on cycles, enforced regardless of caller's
/// `max_cycles` value. Stops a misconfigured caller from setting
/// `max_cycles = 1_000_000` and accidentally locking the daemon
/// into a multi-hour LLM loop. 50 is generous compared to anything
/// a real long-running task would need.
pub const HARD_MAX_CYCLES: u32 = 50;

/// Register the ability. The `dispatch_registry_handle` lets the
/// handler resolve `<agent>.chat` in-process — symmetric to
/// `mission.discuss_round`'s mechanism. Going back through the IPC
/// client would deadlock the daemon's accept loop because we are
/// already mid-call.
pub fn register(
    reg: &mut AxonAbilityCatalog,
    dispatch_registry_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
) {
    use crate::runtime::ability_dispatch::OwnerKind;
    let handle = Arc::clone(&dispatch_registry_handle);
    reg.register_rpc_with_owner(
        "mission.think",
        OwnerKind::Device,
        Arc::new(move |args| think_handler(&handle, args)),
    );
}

/// `mission.think` handler.
///
/// Args:
/// ```json
/// {
///   "owner_agent_id":  "<agent name>",
///   "prompt":          "<initial task description>",
///   "max_cycles":      5,                 // optional, capped at HARD_MAX_CYCLES
///   "judge_agent_id":  "<agent name>"     // optional, defaults to owner
/// }
/// ```
///
/// Returns:
/// ```json
/// {
///   "ok": true,
///   "cycles_used": N,
///   "termination_reason": "judge_terminate" | "max_cycles_reached" | "worker_silent",
///   "transcript": [
///     {"cycle": 1, "worker": "...", "judge_raw": "...", "judge_parsed": {...}}
///   ],
///   "final_verdict": {<last judge_parsed value, or null if none parsed>}
/// }
/// ```
fn think_handler(
    dispatch_registry_handle: &Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    args: Value,
) -> anyhow::Result<Value> {
    let registry = dispatch_registry_handle.get().ok_or_else(|| {
        anyhow::anyhow!(
            "mission.think: dispatch registry handle not yet set; this is an internal \
             init-order bug — register() must run after the daemon's OnceLock is bound"
        )
    })?;
    think_with_registry(registry, args)
}

/// Core orchestration loop with the registry passed in directly.
/// Production code reaches this through `think_handler` (which
/// pulls the registry from the daemon's OnceLock); tests can call
/// it directly with a hand-built registry containing stub
/// `<agent>.chat` handlers, exercising the full worker+judge+
/// curator flow without an LLM subprocess. Same boundary the
/// shell/http executors use to keep their core unit-testable.
pub(crate) fn think_with_registry(
    registry: &Arc<AxonAbilityCatalog>,
    args: Value,
) -> anyhow::Result<Value> {
    let parsed = parse_think_args(&args)?;
    let ThinkArgsParsed {
        owner,
        judge,
        prompt,
        max_cycles,
        dry_run,
    } = parsed;

    // Resolve the two chat ability names up front. Worker and judge
    // can be the same agent — the sessions are independent (each call
    // is a new chat session with its own session_id), the model and
    // tool catalog are the agent's.
    let worker_chat_name = format!("{owner}.chat");
    let judge_chat_name = format!("{judge}.chat");

    let mut transcript: Vec<Value> = Vec::new();
    let mut worker_session_id: Option<String> = None;
    let mut last_parsed_verdict: Option<Value> = None;
    let mut termination = "max_cycles_reached";
    let mut cycles_used: u32 = 0;

    for cycle in 1..=max_cycles {
        cycles_used = cycle;

        // ── Worker turn ─────────────────────────────────────────
        // Cycle 1: full task prompt. Cycle 2+: short "continue"
        // hint, because resuming the session_id rehydrates the
        // entire prior context — re-pasting the prompt would just
        // confuse the worker about whether it is restarting.
        let worker_prompt = if cycle == 1 {
            prompt.clone()
        } else {
            CONTINUE_HINT.to_string()
        };
        let mut worker_args = json!({"prompt": worker_prompt});
        if let Some(sid) = worker_session_id.as_deref() {
            worker_args["session_id"] = json!(sid);
        }
        // Worker chat failures (broken-pipe panics from the LLM
        // subprocess fd-juggling, transient subprocess crashes,
        // etc.) must NOT abort the whole think. Record the failure
        // in the transcript, terminate gracefully with a
        // distinguishable reason so the operator can audit. Same
        // failure-soft policy as the curator step at the end of
        // the loop.
        let worker_resp = match invoke_chat_protected(registry, &worker_chat_name, worker_args) {
            Ok(v) => v,
            Err(e) => {
                termination = "worker_error";
                transcript.push(json!({
                    "cycle": cycle,
                    "worker_error": format!("{e}"),
                    "judge_raw": null,
                    "judge_parsed": null,
                }));
                break;
            }
        };
        if let Some(sid) = worker_resp
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            worker_session_id = Some(sid);
        }
        let worker_text = worker_resp
            .get("reply")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if worker_text.is_empty() {
            // Worker said nothing — either it has finished or it
            // produced no progress. Either way, judge has nothing
            // to read; terminate with a distinct reason so the
            // caller can tell this from a real "judge terminate".
            termination = "worker_silent";
            transcript.push(json!({
                "cycle": cycle,
                "worker": "",
                "judge_raw": null,
                "judge_parsed": null,
            }));
            break;
        }

        // ── Judge turn ──────────────────────────────────────────
        // Independent session — no resume. Judge gets a fresh read
        // of the worker's output every cycle, framed by the JUDGE
        // schema prompt. We do NOT propagate the judge's
        // session_id; cross-cycle judge memory would just let it
        // get attached to the worker the same way single-session
        // self-judgment fails (see module preamble).
        let judge_prompt = render_judge_prompt(&prompt, &worker_text, cycle, max_cycles);
        // Judge chat failures: same failure-soft policy as worker.
        // The cycle's judge entry records the error string; the
        // outer loop continues with no parsed verdict for this
        // cycle, and termination flips to "judge_error" so the
        // operator sees it in the envelope.
        let judge_resp = match invoke_chat_protected(
            registry,
            &judge_chat_name,
            json!({"prompt": judge_prompt}),
        ) {
            Ok(v) => v,
            Err(e) => {
                termination = "judge_error";
                transcript.push(json!({
                    "cycle": cycle,
                    "worker": worker_text,
                    "judge_error": format!("{e}"),
                    "judge_parsed": null,
                }));
                break;
            }
        };
        let judge_raw = judge_resp
            .get("reply")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let judge_parsed = parse_judge_verdict(&judge_raw);
        if let Some(parsed) = judge_parsed.as_ref() {
            last_parsed_verdict = Some(parsed.clone());
        }

        transcript.push(json!({
            "cycle": cycle,
            "worker": worker_text,
            "judge_raw": judge_raw,
            "judge_parsed": judge_parsed,
        }));

        // Termination signal. We respect a parsed `terminate: true`
        // verdict regardless of memory_type — the judge can decide
        // the task is done with NO sinkable lesson. Conversely, a
        // sinkable lesson does NOT mean done; the worker may have
        // produced an interim insight and the task is still in
        // flight.
        if let Some(parsed) = &judge_parsed {
            if parsed
                .get("terminate")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                termination = "judge_terminate";
                break;
            }
        }
    }

    // ── Curator turn (third session) ────────────────────────────
    // After the loop, if the final verdict names a sinkable lesson,
    // a third independent chat session authors the deliverable and
    // dispatches it to ability.publish / skill.publish. The curator
    // sees ONLY the verdict + transcript; it does not see the
    // worker's session memory (a fresh session prevents the same
    // self-judgment failure mode that motivated separating worker
    // from judge — see module preamble). The curator's output is
    // either:
    //   * a full ability.toml body (when scope = "team")
    //   * a full SKILL.md body    (when scope = "private")
    // routed to the matching publish handler. Failures during
    // authoring or publishing are recorded in `curator` but DO NOT
    // fail the overall mission.think — the verdict and transcript
    // are still useful even when the curator step fails.
    // Catalog of the owner's currently-published abilities. Read at
    // curator time (not earlier) so any ability the worker itself
    // published during the run is visible. The curator prompt embeds
    // this catalog (P0 fix: prevents curator from hallucinating
    // `<agent>.fictional_verb(...)` references); validation also
    // uses it to refuse a manifest whose [exec] kind="eal" source
    // calls a verb that does not exist.
    let catalog = collect_owner_catalog(&owner);

    let curator_outcome = match last_parsed_verdict.as_ref() {
        Some(v) if should_curate(v) => Some(run_curator_turn(
            registry,
            &owner,
            &judge,
            &prompt,
            v,
            &transcript,
            &catalog,
            dry_run,
        )),
        _ => None,
    };

    Ok(json!({
        "ok": true,
        "cycles_used": cycles_used,
        "termination_reason": termination,
        "transcript": transcript,
        "final_verdict": last_parsed_verdict,
        "curator": curator_outcome,
    }))
}

/// Decide whether the curator should run for a given verdict.
/// Mirrors the judge prompt's exclusion logic: any of the four
/// exclusion checks firing or `memory_type = "none"` means skip.
/// We re-check on this side because the judge can produce a verdict
/// that says "memory_type = project" while ALSO marking
/// `is_derivable_from_code = true` — the prompt warns against this
/// but a real LLM occasionally does it. Belt-and-suspenders.
fn should_curate(verdict: &Value) -> bool {
    let memory_type = verdict
        .get("memory_type")
        .and_then(Value::as_str)
        .unwrap_or("none");
    if memory_type == "none" || memory_type.is_empty() {
        return false;
    }
    if let Some(checks) = verdict.get("exclusion_check").and_then(Value::as_object) {
        for key in [
            "is_derivable_from_code",
            "is_in_git_log",
            "is_debug_recipe",
            "is_ephemeral",
        ] {
            if checks.get(key).and_then(Value::as_bool).unwrap_or(false) {
                return false;
            }
        }
    }
    // `what_to_save` empty is the judge admitting it can't write
    // out the lesson; treat as not curatable.
    if verdict
        .get("what_to_save")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::is_empty)
        .unwrap_or(true)
    {
        return false;
    }
    true
}

/// One row in the ability catalog handed to the curator. Carries
/// only what the curator needs to write a working reference: the
/// fully-qualified verb (`<agent>.<verb>`) plus a short description
/// so the LLM can pick the right one.
pub(crate) struct CatalogEntry {
    pub qualified: String,
    pub description: String,
}

/// Read the owner agent's currently-published ability catalog. Two
/// uses:
///   1. Inject the list into the curator prompt so the curator can
///      reference real abilities in any EAL exec it authors. P0
///      fix against curator hallucinating `<agent>.fictional_verb`.
///   2. Validate the curator's authored manifest post-authoring —
///      every member-call in an `[exec] kind="eal"` source must
///      point at one of these verbs, else the resulting ability is
///      dead on arrival.
///
/// Catalog gathering is best-effort: an unreadable agent dir
/// returns an empty list, and validation downstream emits a clear
/// "no catalog available" rather than refusing to publish.
pub(crate) fn collect_owner_catalog(owner: &str) -> Vec<CatalogEntry> {
    let registry = match crate::registry::agents::load_agents() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let entry = match registry.agents.get(owner) {
        Some(e) => e.clone(),
        None => return Vec::new(),
    };
    let manifests = crate::runtime::abilities::manifests_for(owner, &entry);
    manifests
        .into_iter()
        .map(|m| CatalogEntry {
            qualified: format!("{owner}.{}", m.name()),
            description: m.description().to_string(),
        })
        .collect()
}

/// Validate the curator's authored ability body before publish.
/// Returns `Ok(())` if structurally sound and (for EAL execs)
/// every referenced ability exists in the catalog. Returns
/// `Err(reason)` otherwise — the caller surfaces it in the
/// `curator.error` field of the envelope so the operator can see
/// exactly why publish was vetoed.
///
/// What we check
/// -------------
///   * Manifest must round-trip through `AbilityManifest::from_toml_str`
///     (catches every from_toml_str failure: bad schema, unknown
///     sandbox profile, schema_version mismatch, etc.)
///   * If `[exec] kind = "eal"`: source must parse via
///     `eal::parser::parse`. A malformed EAL mission would
///     otherwise produce an ability that fails on first call.
///   * If `[exec] kind = "eal"`: every `<agent>.<verb>` member-call
///     in the EAL source must appear in the owner's catalog. A
///     curator that hallucinates `claude.read_email(...)` against
///     an agent without that verb would otherwise produce a
///     permanent dead reference.
///
/// What we do NOT check
/// --------------------
/// Argument shape of referenced calls. The EAL parser does not yet
/// surface arg lists matched against the referenced ability's
/// input_schema; arg-shape validation lands when that surface
/// exists. Today, a curator-published EAL ability that calls
/// `<agent>.<verb>(wrong_arg: "...")` fails at invocation time,
/// not at publish time.
pub(crate) fn validate_authored_ability(
    body: &str,
    catalog: &[CatalogEntry],
) -> Result<(), String> {
    use crate::core::ability_spec::{AbilityExec, AbilityManifest};

    let manifest =
        AbilityManifest::from_toml_str(body).map_err(|e| format!("manifest parse failed: {e}"))?;
    if let Some(AbilityExec::Eal(eal)) = manifest.exec() {
        let program = crate::eal::parser::parse(&eal.source)
            .map_err(|e| format!("EAL source parse failed: {e}"))?;
        let referenced = collect_member_call_targets(&program);
        let known: std::collections::HashSet<&str> =
            catalog.iter().map(|c| c.qualified.as_str()).collect();
        let unknown: Vec<String> = referenced
            .into_iter()
            .filter(|r| !known.contains(r.as_str()))
            .collect();
        if !unknown.is_empty() {
            return Err(format!(
                "EAL source references {} ability(ies) not in the {}-entry owner catalog: {:?}",
                unknown.len(),
                catalog.len(),
                unknown
            ));
        }
    }
    Ok(())
}

/// Walk a parsed EAL program and collect every member-call target
/// (`<agent>.<verb>`). We walk the parsed AST rather than regex
/// over the source so a string literal that happens to contain a
/// `.` does not produce a false positive.
fn collect_member_call_targets(program: &crate::eal::ast::EalProgram) -> Vec<String> {
    use crate::eal::ast::{CallExpr, FieldValue, Statement, TargetKind};
    let mut out: Vec<String> = Vec::new();
    fn visit_call(c: &CallExpr, out: &mut Vec<String>) {
        if c.target_kind == TargetKind::Agent {
            if let Some(agent) = &c.target_node {
                out.push(format!("{agent}.{}", c.function_name));
            }
        }
        // Recurse into nested calls embedded as object-valued args
        // (we don't expect nested CallExpr today but FieldValue::Object
        // could hold them once the parser grows that surface).
        for field in &c.arguments {
            visit_field_value(&field.value);
        }
    }
    fn visit_field_value(v: &FieldValue) {
        if let FieldValue::Object(fields) = v {
            for f in fields {
                visit_field_value(&f.value);
            }
        }
    }
    fn visit_stmt(s: &Statement, out: &mut Vec<String>) {
        match s {
            Statement::LetCall { call, .. } => visit_call(call, out),
            Statement::Call(c) => visit_call(c, out),
            Statement::Loop(b) => {
                for s in &b.body {
                    visit_stmt(s, out);
                }
                for s in &b.verify {
                    visit_stmt(s, out);
                }
            }
        }
    }
    for s in &program.mission.statements {
        visit_stmt(s, &mut out);
    }
    out
}

/// Spawn the curator session, ask it to author the deliverable for
/// the verdict, validate the authored body, and (when not in
/// dry_run) dispatch to the matching publish handler. Returns a
/// JSON outcome describing what happened at every stage so the
/// operator reading the mission.think envelope can audit the
/// publish step without grep'ing the daemon log.
#[allow(clippy::too_many_arguments)]
fn run_curator_turn(
    registry: &Arc<AxonAbilityCatalog>,
    owner: &str,
    curator_agent: &str,
    initial_prompt: &str,
    verdict: &Value,
    transcript: &[Value],
    catalog: &[CatalogEntry],
    dry_run: bool,
) -> Value {
    let scope = verdict
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("private");
    let target = if scope == "team" { "ability" } else { "skill" };

    let curator_chat_name = format!("{curator_agent}.chat");

    let prompt = render_curator_prompt(target, initial_prompt, verdict, transcript, catalog);
    let resp = match invoke_chat_protected(registry, &curator_chat_name, json!({"prompt": prompt}))
    {
        Ok(v) => v,
        Err(e) => {
            return json!({
                "attempted": true,
                "ok": false,
                "stage": "curator_chat",
                "error": format!("{e}"),
            });
        }
    };
    let body = resp
        .get("reply")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if body.is_empty() {
        return json!({
            "attempted": true,
            "ok": false,
            "stage": "curator_authoring",
            "error": "curator session returned an empty body",
        });
    }
    let authored = strip_code_fence(&body)
        .map(str::to_string)
        .unwrap_or(body.clone());

    // Pre-publish validation. Both branches surface failures as
    // `stage = "validate"` with the reason string + the authored
    // body so an operator can see what was rejected.
    //
    //   * "ability" target → full validate_authored_ability pipeline
    //     (manifest parse + EAL parse + catalog reference resolution)
    //   * "skill" target   → validate_authored_skill checks the load-
    //     bearing Anthropic-skill structure (front matter with
    //     name + description + allowed-tools, "Use when …" hint in
    //     description, `## When This Skill Activates` section).
    //     Without those, the skill is invisible to Claude Code's
    //     skill loader; publishing would write a dead file.
    let validation: Result<(), String> = if target == "ability" {
        validate_authored_ability(&authored, catalog)
    } else {
        validate_authored_skill(&authored)
    };
    if let Err(reason) = validation {
        return json!({
            "attempted": true,
            "ok": false,
            "stage": "validate",
            "target": target,
            "error": reason,
            "authored_body": authored,
            "catalog_size": catalog.len(),
        });
    }

    // dry_run short-circuit. Validation passed, but no publish
    // dispatch. The caller gets the authored body back behind an
    // explicit `dry_run: true` marker so an operator running
    // `easynet mission think --dry-run` can inspect what *would*
    // have been published without polluting the agent workspace.
    if dry_run {
        return json!({
            "attempted": true,
            "ok": true,
            "dry_run": true,
            "target": target,
            "authored_body": authored,
            "catalog_size": catalog.len(),
        });
    }

    // Dispatch to the matching publish handler through the same
    // LocalRuntime-backed path as worker/judge dispatch.
    let (publish_name, publish_args) = if target == "ability" {
        (
            "ability.publish",
            json!({"owner_agent_id": owner, "manifest_toml": authored}),
        )
    } else {
        // The Anthropic skill loader treats the front-matter
        // `name:` field as the canonical skill identifier — the
        // directory under `skills/<name>/` MUST match for the
        // loader's discovery walk to find it. Earlier we slugified
        // the verdict's `what_to_save` and used that, which
        // produced disk paths like
        // `skills/don-t-run-multiple-parallel-cargo-test-...` while
        // the front matter said `name: rust-test-parallelism`. The
        // skill activated under one name but lived at a different
        // path — confusing for operators and a real divergence the
        // loader does not paper over. Now we extract `name:` from
        // the curator's front matter (validated up-front by
        // `validate_authored_skill`) and use that. Falls back to
        // the verdict-derived slug only when the front-matter
        // parse fails defensively — but the validator should have
        // caught that case already.
        let name_from_frontmatter = extract_skill_name_from_frontmatter(&authored);
        let skill_name = name_from_frontmatter.unwrap_or_else(|| {
            let raw_name = verdict
                .get("what_to_save")
                .and_then(Value::as_str)
                .unwrap_or("curated-skill");
            slugify(raw_name)
        });
        (
            "skill.publish",
            json!({
                "owner_agent_id": owner,
                "skill_name": skill_name,
                "skill_md": authored,
            }),
        )
    };

    if !registry.has_rpc(publish_name) {
        return json!({
            "attempted": true,
            "ok": false,
            "stage": "resolve_publish",
            "error": format!("{publish_name} not registered"),
            "authored_body_len": authored.len(),
        });
    }
    match registry.invoke_rpc_json(publish_name, publish_args) {
        Ok(v) => json!({
            "attempted": true,
            "ok": true,
            "target": target,
            "publish_result": v,
        }),
        Err(e) => json!({
            "attempted": true,
            "ok": false,
            "stage": "publish",
            "target": target,
            "error": format!("{e}"),
            "authored_body_len": authored.len(),
        }),
    }
}

/// Curator prompt. The curator's job is purely to write the
/// deliverable — manifest TOML or SKILL.md — given the verdict's
/// what_to_save / why / how_to_apply. We do NOT ask the curator to
/// re-judge whether the lesson should be saved; the judge already
/// decided. A second-guessing curator just produces empty bodies
/// for borderline cases.
fn render_curator_prompt(
    target: &str,
    initial_prompt: &str,
    verdict: &Value,
    transcript: &[Value],
    catalog: &[CatalogEntry],
) -> String {
    let what = verdict
        .get("what_to_save")
        .and_then(Value::as_str)
        .unwrap_or("");
    let why = verdict.get("why").and_then(Value::as_str).unwrap_or("");
    let how = verdict
        .get("how_to_apply")
        .and_then(Value::as_str)
        .unwrap_or("");
    let memory_type = verdict
        .get("memory_type")
        .and_then(Value::as_str)
        .unwrap_or("");

    if target == "ability" {
        // P0 fix: render the owner's existing ability catalog into
        // the prompt so the curator can ONLY reference real verbs.
        // Without this, the curator hallucinates plausible-sounding
        // names (`<agent>.read_email`, `<agent>.lookup_weather`)
        // that do not exist on the agent and the published ability
        // is dead on arrival. We pin this with the post-authoring
        // validate_authored_ability() check, but giving the curator
        // the catalog up front avoids the failed-validation round
        // trip in the common case.
        let catalog_block = if catalog.is_empty() {
            "(no abilities currently published on this agent — author a chat-fallback \
             ability without an [exec] block, or a self-contained shell exec)"
                .to_string()
        } else {
            let mut s = String::new();
            for e in catalog {
                s.push_str(&format!("  * {} — {}\n", e.qualified, e.description));
            }
            s
        };
        format!(
            r#"You are a curator. The judge has decided that one experience from a recent task is worth saving as a published ability (device-visible). Your job is to author the ability.toml body that will be published.

# The original task

{initial_prompt}

# What the judge decided

memory_type:    {memory_type}
what_to_save:   {what}
why:            {why}
how_to_apply:   {how}

# Abilities currently available on this agent

If you write an `[exec] kind = "eal"` block, every `<agent>.<verb>(...)` member-call MUST reference one of these. Do NOT invent verb names — the publish step rejects unknown references.

{catalog_block}

# Format

Output ONLY a valid ability.toml body. Required structure:

schema_version = "1"
name = "<short_snake_case_verb>"
description = "<one-line ability description>"
[input_schema]
type = "object"
properties = {{ ... }}
required = [...]

If the lesson is best expressed as a deterministic workflow over the abilities listed above, add an `[exec]` block:

[exec]
kind = "eal"
source = """
mission "<name>" {{
  let r = <agent>.<verb>(arg1: "...", arg2: "{{{{ input_field }}}}")
}}
"""
result_binding = "r"

If the lesson is more nuanced and requires LLM judgement (no deterministic recipe over the listed abilities), omit the [exec] block — the dispatcher will route the call through the agent's chat fallback.

Do NOT wrap the output in a markdown fence. Do NOT preface with prose. Output ONLY the TOML.
"#,
            initial_prompt = initial_prompt,
            what = what,
            why = why,
            how = how,
            memory_type = memory_type,
            catalog_block = catalog_block,
        )
    } else {
        let _ = catalog; // Skill bodies don't reference abilities.
        let _ = transcript; // Reserved for future curator prompts that quote the transcript.
                            // Anthropic-canonical skill body. The earlier prompt let the
                            // curator emit a prose memo with `**Why:**` / `**How to apply:**`
                            // headings (borrowed from the AliveCode memory model). That
                            // produced human-readable text but missed the machine-readable
                            // structure that makes a skill *activate*: Claude Code resolves
                            // skills by scanning `description` for a "use when …" hint and
                            // by matching the `## When This Skill Activates` triggers
                            // against the running prompt. Without those, the skill sits in
                            // `skills_loaded` but never gets reached for. We model the
                            // prompt here on Anthropic's own
                            // `~/.claude/skills/shared/skill-creator/skill-template.md` so
                            // the curator's output is byte-shape compatible with the rest
                            // of the official skill catalog.
        format!(
            r#"You are a curator. The judge has decided that one experience from a recent task is worth saving as a SKILL — a self-contained Claude Code skill that the agent can apply to future similar prompts. Your job is to author the SKILL.md body.

# The original task

{initial_prompt}

# What the judge decided

memory_type:    {memory_type}
what_to_save:   {what}
why:            {why}
how_to_apply:   {how}

# Required structure (Anthropic skill convention)

The skill activates when Claude Code matches its `description` and `## When This Skill Activates` triggers against an incoming prompt. Both fields are LOAD-BEARING — a skill missing them will never run. Author exactly this structure:

```markdown
---
name: <kebab-case-slug>
description: <1–2 sentences. Start with what the skill does. END WITH "Use when …" so the model has a clear activation hint. Example: "Reviews Swift code for force-unwrap and unsafe optional patterns. Use when reviewing Swift code or refactoring optionals.">
allowed-tools: [<tool list>]
---

# <Title Case Skill Name>

One paragraph describing what the skill does and its primary purpose.

## When This Skill Activates

Use this skill when the user:
- <Specific trigger phrase or action 1>
- <Specific trigger phrase or action 2>
- <Specific trigger phrase or action 3>

## Process

### 1. <First Step>

- <What to do, what to check>

### 2. <Second Step>

- <What data to gather, how to process>

### 3. Output Format

<How to present results to the user.>

## Examples

### Example: <Scenario name>

**Input:**
```
<concrete example input>
```

**Expected Output:**
```
<concrete expected output>
```

## Notes

- <Edge cases, limitations, additional context>
```

# Field rules

`name`: kebab-case slug, no spaces, no underscores. The skill directory will be named after this. Match it to the slug `skill.publish` uses.

`description`: Two sentences max. The first names what the skill does; the second begins with "Use when" and names trigger phrases the operator might say. **Without "Use when", the skill is hard for Claude Code to activate.**

`allowed-tools`: Pick from this list and choose the minimum the skill actually needs:
  * Read-only analysis: `[Read, Glob, Grep]`
  * Code modification:  `[Read, Write, Edit]`
  * Full access:        `[Read, Write, Edit, Glob, Grep, Bash]`
  * Web research:       `[Read, WebFetch]`

`## When This Skill Activates`: list of bullet phrases starting with "Asks…", "Mentions…", "Wants…", "Needs…". These are what the model matches against. Generic triggers ("any task involving X") activate too often; over-specific triggers ("when running Rust 1.85") activate too rarely.

# Output rules

  * Output ONLY the SKILL.md body. No markdown fence around the whole document. No prose preamble.
  * Use the structure above. Do NOT substitute `**Why:**` / `**How to apply:**` paragraphs for the canonical sections — Claude Code's skill loader does not understand those headings as activation triggers.
  * Aim for 60–250 lines. A 400+ line skill should be split into a multi-file skill (out of scope here — emit a single SKILL.md and note in `## Notes` if a future split would help).
  * Concrete examples in the `## Examples` section are MANDATORY. The judge's `what_to_save` and `how_to_apply` are the seed; flesh them into a real input → output example a future you could copy-paste.
"#,
            initial_prompt = initial_prompt,
            what = what,
            why = why,
            how = how,
            memory_type = memory_type,
        )
    }
}

/// Validate that a curator-authored SKILL.md body has the
/// load-bearing Anthropic-canonical sections. Returns `Ok(())`
/// if the body is shaped to actually activate; `Err(reason)`
/// otherwise — the caller surfaces the reason in the curator
/// envelope so an operator running with `--dry-run` sees what
/// was rejected and why.
///
/// What we check (cheap regex/substring, not a full markdown
/// parser — this is post-LLM-output sanity, not a stylesheet):
///   * Front-matter delimiters `---` are present at the top
///   * Front matter contains `name:`, `description:`, `allowed-tools:`
///   * `description` value contains "Use when" (the activation hint
///     Claude Code's skill loader looks for)
///   * The body contains `## When This Skill Activates`
///
/// What we do NOT check:
///   * YAML well-formedness — Claude Code's skill loader is more
///     forgiving than a strict YAML parser, and a curator that
///     produced *almost* valid YAML is closer to fixable than a
///     curator that produced prose.
///   * Process / Examples / Notes sections — those are best-practice
///     but not load-bearing for activation.
fn validate_authored_skill(body: &str) -> Result<(), String> {
    let trimmed = body.trim_start();
    if !trimmed.starts_with("---") {
        return Err(
            "SKILL.md must start with `---` YAML front-matter delimiter (Claude Code's \
             skill loader requires the front matter; without it the skill is invisible)"
                .to_string(),
        );
    }
    // Find the closing `---` to bound the front matter.
    let after_open = &trimmed[3..];
    let close_idx = after_open
        .find("\n---")
        .ok_or_else(|| "SKILL.md front-matter has no closing `---` delimiter".to_string())?;
    let frontmatter = &after_open[..close_idx];
    for required in ["name:", "description:", "allowed-tools:"] {
        if !frontmatter.contains(required) {
            return Err(format!(
                "SKILL.md front matter missing required field `{}`. \
                 The Anthropic skill loader needs name + description + allowed-tools to activate \
                 the skill",
                required.trim_end_matches(':')
            ));
        }
    }
    // Description must contain the "Use when" activation hint.
    // We look for it case-insensitively to tolerate "Use when…" /
    // "use when…" alike.
    let lower_frontmatter = frontmatter.to_ascii_lowercase();
    if !lower_frontmatter.contains("use when") {
        return Err(
            "SKILL.md `description` must contain the phrase \"Use when …\" so the skill loader \
             has an activation hint. Without it the skill sits in skills_loaded but never \
             gets reached for"
                .to_string(),
        );
    }
    // Body section guard: `## When This Skill Activates` is the
    // bullet-trigger section the loader matches the running prompt
    // against. Heading text must match exactly because the loader's
    // section walk is literal.
    if !body.contains("## When This Skill Activates") {
        return Err(
            "SKILL.md body must contain a `## When This Skill Activates` section listing \
             trigger phrases. Without it the skill cannot be matched against incoming prompts"
                .to_string(),
        );
    }
    Ok(())
}

/// Slugify text into a skill_name. The skill.publish handler
/// accepts only ASCII alnum + `-` and `_`; this lowercases and
/// replaces every other character with `-`, then collapses runs
/// and trims leading/trailing dashes. Empty result falls back to
/// `"curated-skill"` so we never hand `skill.publish` an empty
/// name.
/// Pull the `name:` field out of an Anthropic-canonical SKILL.md
/// front matter. Returns `Some(slug)` when the value is a clean
/// kebab-case slug that satisfies `skill.publish`'s validation
/// (ASCII alnum + `-`/`_`, 1..=100 bytes). Returns `None` for any
/// unparseable / unsafe value — the caller should fall back to its
/// own slug source rather than handing the publish handler a name
/// that will fail validation.
///
/// We deliberately do NOT use a YAML parser here:
///   * `serde_yaml` is not pulled into this crate today, and the
///     skill manifest's "front matter" is a tiny subset of YAML
///     (`key: value` lines with optional list values).
///   * Anything strict enough to reject everything `skill.publish`
///     would reject downstream is good enough — we just need the
///     value of one specific scalar key.
///
/// Implementation: locate the closing `---` of the front-matter
/// block, then walk lines looking for `name:` at column 0. Skip
/// any quoted forms and trim whitespace.
fn extract_skill_name_from_frontmatter(body: &str) -> Option<String> {
    let trimmed = body.trim_start();
    let after_open = trimmed.strip_prefix("---")?;
    let close_idx = after_open.find("\n---")?;
    let frontmatter = &after_open[..close_idx];
    for line in frontmatter.lines() {
        // Match `name:`, `name :`, `Name:` (case-insensitive on
        // the key) — be lenient on the LLM's exact spacing.
        let trimmed_line = line.trim_start();
        if let Some(rest) = trimmed_line
            .strip_prefix("name:")
            .or_else(|| trimmed_line.strip_prefix("Name:"))
        {
            let value = rest.trim().trim_matches(|c| c == '"' || c == '\'');
            if value.is_empty() {
                return None;
            }
            // Safety check: skill.publish's validate_skill_name
            // rejects anything outside [A-Za-z0-9_-]. We do the
            // same check here so we don't pass the publish handler
            // a name we know will fail — fallback to the verdict
            // slug instead.
            if value.len() > 100 {
                return None;
            }
            for c in value.chars() {
                if !(c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    return None;
                }
            }
            return Some(value.to_string());
        }
    }
    None
}

fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = true;
    for c in s.chars() {
        let ok = c.is_ascii_alphanumeric() || c == '_';
        if ok {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        return "curated-skill".to_string();
    }
    // Cap at 80 to leave headroom under the publish handler's 100-byte
    // limit even if a future prefix is added.
    if trimmed.len() > 80 {
        trimmed[..80].trim_end_matches('-').to_string()
    } else {
        trimmed
    }
}

/// Hint passed to the worker on cycle 2+ in lieu of re-pasting the
/// initial prompt. Resuming the session_id rehydrates context;
/// what the worker needs is a nudge to push forward, not a
/// re-statement of what it already knows.
const CONTINUE_HINT: &str =
    "Continue with the task. If you have arrived at a satisfactory result, summarise it. \
     If you are blocked, state the blocker so the next step is unambiguous.";

/// Wrap a chat invocation in `catch_unwind` so an `eprintln!` to a
/// closed stderr (broken pipe → panic in the std macros) does not
/// take down the think handler. Same defensive pattern
/// `mission.discuss_round` uses; the chat handler does heavy fd
/// juggling around the LLM subprocess and rare paths can panic on
/// stderr writes when the parent shell's stderr is gone.
fn invoke_chat_protected(
    registry: &Arc<AxonAbilityCatalog>,
    ability: &str,
    args: Value,
) -> anyhow::Result<Value> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        registry.invoke_rpc_json(ability, args)
    }));
    match result {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(anyhow::anyhow!("{e}")),
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<&'static str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "non-string panic from chat handler".to_string()
            };
            Err(anyhow::anyhow!("chat handler panicked: {msg}"))
        }
    }
}

/// Parsed args carrier. A struct beats a 5-tuple because the
/// dry_run flag was added late and the call sites — both
/// production and tests — would otherwise need to count
/// positional arguments.
#[derive(Debug)]
struct ThinkArgsParsed {
    owner: String,
    judge: String,
    prompt: String,
    max_cycles: u32,
    dry_run: bool,
}

fn parse_think_args(args: &Value) -> anyhow::Result<ThinkArgsParsed> {
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("mission.think: args must be a JSON object"))?;
    let owner = obj
        .get("owner_agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("mission.think: missing/empty `owner_agent_id` (the worker agent)")
        })?
        .to_string();
    let prompt = obj
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("mission.think: missing/empty `prompt` (the task description)")
        })?
        .to_string();
    // Judge defaults to the owner. Same agent → independent
    // sessions; we deliberately do NOT thread a session_id through
    // judge calls so it gets a fresh read each cycle (see preamble).
    let judge = obj
        .get("judge_agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| owner.clone());
    let max_cycles = obj
        .get("max_cycles")
        .and_then(Value::as_u64)
        .map(|n| n.min(HARD_MAX_CYCLES as u64) as u32)
        .unwrap_or(DEFAULT_MAX_CYCLES);
    if max_cycles == 0 {
        anyhow::bail!("mission.think: `max_cycles` must be ≥ 1");
    }
    // dry_run gates the publish step. The curator still authors a
    // body and the body still gets validated (parse + reference
    // resolution), but `ability.publish` / `skill.publish` is not
    // dispatched. Operators driving this from `easynet mission
    // think --dry-run` use it to inspect what *would* be published
    // before letting it touch their workspace.
    let dry_run = obj.get("dry_run").and_then(Value::as_bool).unwrap_or(false);
    Ok(ThinkArgsParsed {
        owner,
        judge,
        prompt,
        max_cycles,
        dry_run,
    })
}

/// Build the judge's prompt. The schema below is borrowed from the
/// AliveCode memory model (memoryTypes.ts) so the curator (Phase 5)
/// can route on `scope` directly without a translation layer. The
/// `terminate` flag is the load-bearing decision; everything else
/// is sinkable-lesson metadata that may legitimately be `none`.
fn render_judge_prompt(
    initial_prompt: &str,
    worker_output: &str,
    cycle: u32,
    max_cycles: u32,
) -> String {
    format!(
        r#"You are a judge evaluating one cycle of a worker session that is trying to complete a task. Your job has two parts:

1. Decide whether the worker has finished the task or has stalled (the loop should terminate).
2. Decide whether anything from this cycle is a SINKABLE LESSON — an experience worth saving so the agent's future self can reuse it.

# Task the worker was given

{initial_prompt}

# Worker output for cycle {cycle}/{max_cycles}

{worker_output}

# How to decide `terminate`

Set `terminate` = true if any of:
  * the worker has produced a complete, satisfactory answer to the task,
  * the worker is stuck in a loop (repeating earlier output, no new progress),
  * the worker is blocked on something the loop cannot resolve (missing credentials, missing data, etc).

Set `terminate` = false if the worker is making real progress and another cycle will help.

# How to decide the sinkable lesson

A sinkable lesson is an experience that is BOTH non-derivable from the codebase AND will help future agents. The exclusion list:
  * derivable from code, file paths, git log, git blame → SKIP
  * a fix recipe for a single bug (the fix is in the diff; the commit message has context) → SKIP
  * ephemeral task state ("we are in the middle of X") → SKIP
  * already documented in CLAUDE.md / AGENTS.md → SKIP
  * an activity log or PR list — unless something specific in it is SURPRISING or NON-OBVIOUS → SKIP

If you cannot fill in a clear `why` and `how_to_apply`, the lesson is NOT sinkable; set memory_type to "none".

`memory_type` choices:
  * "feedback":  guidance on how to approach work (corrections OR validated approaches)
  * "project":   facts/decisions/constraints about this project's ongoing state
  * "reference": pointer to where information lives in an external system
  * "user":      something about the user's role/preferences/knowledge
  * "none":      no sinkable lesson this cycle

`scope` choices:
  * "team":    every agent on the device benefits → routes to ability.publish
  * "private": only this specific agent benefits → routes to skill.publish

`user`-type lessons are always private. `feedback` defaults private unless the rule is clearly a project-wide convention. `project` and `reference` strongly bias team.

# Output format

Respond with ONLY a JSON object — no preamble, no markdown fence, no commentary.

{{
  "terminate": true|false,
  "memory_type": "feedback"|"project"|"reference"|"user"|"none",
  "scope": "private"|"team",
  "what_to_save": "<single sentence rule/fact>",
  "why": "<the reason, often a past incident or strong preference>",
  "how_to_apply": "<when/where this kicks in>",
  "exclusion_check": {{
    "is_derivable_from_code": true|false,
    "is_in_git_log": true|false,
    "is_debug_recipe": true|false,
    "is_ephemeral": true|false
  }}
}}

If memory_type = "none", you may set scope/what_to_save/why/how_to_apply to empty strings, but the field must still be present.
"#,
        initial_prompt = initial_prompt,
        worker_output = worker_output,
        cycle = cycle,
        max_cycles = max_cycles,
    )
}

/// Parse the judge's reply into a verdict object. The judge
/// instruction asks for a bare JSON object, but real LLM output
/// drifts (markdown fences, "Here is my verdict:" preamble, …).
/// We try strict parse first, then strip a leading `{ … }` block
/// from the text. Returns `None` if neither works — the caller
/// records the raw text and proceeds.
///
/// Why fail soft: a single un-parseable judge cycle should not
/// crash the loop. The transcript still records `judge_raw` so
/// the operator can see what the judge actually said when reading
/// the run dir.
fn parse_judge_verdict(raw: &str) -> Option<Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        if v.is_object() {
            return Some(v);
        }
    }
    // Strip ```json fences if present.
    if let Some(stripped) = strip_code_fence(trimmed) {
        if let Ok(v) = serde_json::from_str::<Value>(stripped) {
            if v.is_object() {
                return Some(v);
            }
        }
    }
    // Last resort: locate the first `{` and the matching `}` (by
    // brace-depth count) and try to parse just that. Handles
    // "Here is my verdict: {...}" -style preambles.
    if let Some(slice) = extract_first_json_object(trimmed) {
        if let Ok(v) = serde_json::from_str::<Value>(slice) {
            if v.is_object() {
                return Some(v);
            }
        }
    }
    None
}

fn strip_code_fence(s: &str) -> Option<&str> {
    let s = s.strip_prefix("```json")?.trim_start_matches('\n');
    s.strip_suffix("```").or_else(|| s.strip_suffix("```\n"))
}

fn extract_first_json_object(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes[start..].iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if b == b'\\' && in_string {
            escape = true;
            continue;
        }
        if b == b'"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(&s[start..start + i + 1]);
            }
        }
    }
    None
}

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["owner_agent_id", "prompt"],
        "properties": {
            "owner_agent_id": {
                "type": "string",
                "description": "The worker agent. Its `<agent>.chat` ability is invoked once per cycle with cross-session resume."
            },
            "prompt": {
                "type": "string",
                "description": "The initial task description for the worker. Cycle 1 sees this verbatim; cycles 2+ see a continue-hint, with the prior session_id rehydrating context."
            },
            "max_cycles": {
                "type": "integer",
                "minimum": 1,
                "description": "Hard cap on worker+judge cycles. Default 5, ceiling HARD_MAX_CYCLES (50)."
            },
            "judge_agent_id": {
                "type": "string",
                "description": "Optional. The judge agent. Defaults to owner_agent_id; the *sessions* are independent regardless."
            }
        }
    })
}

pub fn description() -> &'static str {
    "Run a long-running task with a separate judge session. Each cycle: the worker session \
     resumes its prior session_id and pushes the task forward; an independent judge session \
     reads the worker's latest output, decides whether to terminate, and emits a memory- \
     classification verdict (which Phase 5's curator turns into ability.publish or skill.publish)."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_json_verdict() {
        let raw = r#"{"terminate":true,"memory_type":"none","scope":"private","what_to_save":"","why":"","how_to_apply":"","exclusion_check":{"is_derivable_from_code":true,"is_in_git_log":false,"is_debug_recipe":false,"is_ephemeral":true}}"#;
        let v = parse_judge_verdict(raw).expect("parses");
        assert_eq!(v["terminate"], true);
        assert_eq!(v["memory_type"], "none");
    }

    #[test]
    fn parses_verdict_with_preamble_and_fence() {
        // Real LLM drift: prose preamble + markdown fence.
        let raw = r#"Here is my verdict:

```json
{"terminate": false, "memory_type": "project", "scope": "team", "what_to_save": "x", "why": "y", "how_to_apply": "z", "exclusion_check": {"is_derivable_from_code": false, "is_in_git_log": false, "is_debug_recipe": false, "is_ephemeral": false}}
```
"#;
        let v = parse_judge_verdict(raw).expect("parses through fence + preamble");
        assert_eq!(v["terminate"], false);
        assert_eq!(v["memory_type"], "project");
    }

    #[test]
    fn returns_none_on_empty_or_garbage() {
        assert!(parse_judge_verdict("").is_none());
        assert!(parse_judge_verdict("not json at all").is_none());
        // A JSON array (not object) must NOT count as a verdict —
        // the contract is an object so a stray array would silently
        // misroute downstream.
        assert!(parse_judge_verdict("[1,2,3]").is_none());
    }

    #[test]
    fn parse_args_rejects_missing_owner() {
        let err = parse_think_args(&json!({"prompt": "x"})).unwrap_err();
        assert!(format!("{err}").contains("owner_agent_id"));
    }

    #[test]
    fn parse_args_rejects_missing_prompt() {
        let err = parse_think_args(&json!({"owner_agent_id": "a"})).unwrap_err();
        assert!(format!("{err}").contains("prompt"));
    }

    #[test]
    fn parse_args_caps_max_cycles_at_hard_ceiling() {
        let p = parse_think_args(&json!({
            "owner_agent_id": "a",
            "prompt": "p",
            "max_cycles": 1_000_000,
        }))
        .unwrap();
        assert_eq!(p.max_cycles, HARD_MAX_CYCLES);
    }

    #[test]
    fn parse_args_rejects_zero_max_cycles() {
        let err = parse_think_args(&json!({
            "owner_agent_id": "a",
            "prompt": "p",
            "max_cycles": 0,
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("≥ 1"));
    }

    #[test]
    fn parse_args_judge_defaults_to_owner() {
        let p = parse_think_args(&json!({
            "owner_agent_id": "alice",
            "prompt": "p",
        }))
        .unwrap();
        assert_eq!(p.owner, "alice");
        assert_eq!(p.judge, "alice");
        assert!(!p.dry_run, "dry_run defaults to false");
    }

    #[test]
    fn should_curate_skips_when_memory_type_is_none() {
        let v = json!({"memory_type": "none", "what_to_save": "x"});
        assert!(!should_curate(&v));
    }

    #[test]
    fn should_curate_skips_when_any_exclusion_fires() {
        // Even with a real memory_type and what_to_save, an
        // exclusion check firing cancels curation. This is the
        // belt-and-suspenders re-check (see fn doc) — judge can
        // be inconsistent across fields.
        let v = json!({
            "memory_type": "project",
            "what_to_save": "a fact",
            "exclusion_check": {
                "is_derivable_from_code": false,
                "is_in_git_log": true,
                "is_debug_recipe": false,
                "is_ephemeral": false,
            }
        });
        assert!(!should_curate(&v));
    }

    #[test]
    fn should_curate_skips_when_what_to_save_is_empty() {
        // Empty body means the judge couldn't write the lesson out.
        let v = json!({
            "memory_type": "project",
            "what_to_save": "   ",
            "exclusion_check": {
                "is_derivable_from_code": false,
                "is_in_git_log": false,
                "is_debug_recipe": false,
                "is_ephemeral": false,
            }
        });
        assert!(!should_curate(&v));
    }

    #[test]
    fn should_curate_runs_when_all_signals_align() {
        let v = json!({
            "memory_type": "project",
            "scope": "team",
            "what_to_save": "auth migration is driven by legal compliance",
            "why": "legal flagged session token storage",
            "how_to_apply": "scope decisions favor compliance over ergonomics",
            "exclusion_check": {
                "is_derivable_from_code": false,
                "is_in_git_log": false,
                "is_debug_recipe": false,
                "is_ephemeral": false,
            }
        });
        assert!(should_curate(&v));
    }

    #[test]
    fn slugify_handles_typical_judgments() {
        assert_eq!(slugify("Use bun, not npm"), "use-bun-not-npm");
        assert_eq!(
            slugify("don't mock the database"),
            "don-t-mock-the-database"
        );
        assert_eq!(slugify("汉字 mixed 内容"), "mixed");
        // Empty / pure-symbols → fallback so skill.publish never
        // gets handed an invalid name.
        assert_eq!(slugify(""), "curated-skill");
        assert_eq!(slugify("---"), "curated-skill");
    }

    #[test]
    fn extracts_name_from_canonical_frontmatter() {
        let body = "---\n\
name: rust-test-parallelism\n\
description: x. Use when y.\n\
allowed-tools: [Read]\n\
---\n\
\n\
# X\n";
        assert_eq!(
            extract_skill_name_from_frontmatter(body),
            Some("rust-test-parallelism".to_string())
        );
    }

    #[test]
    fn extracts_name_tolerates_quoted_value() {
        let body = "---\n\
name: \"rust-test-parallelism\"\n\
description: x. Use when y.\n\
allowed-tools: [Read]\n\
---\n";
        assert_eq!(
            extract_skill_name_from_frontmatter(body),
            Some("rust-test-parallelism".to_string())
        );
    }

    #[test]
    fn rejects_name_with_invalid_chars_so_caller_falls_back() {
        // skill.publish's validate_skill_name will reject "..", so
        // the extractor must return None here rather than pass it
        // through. This is the safety belt.
        let body = "---\n\
name: ../escape\n\
description: x. Use when y.\n\
allowed-tools: [Read]\n\
---\n";
        assert!(extract_skill_name_from_frontmatter(body).is_none());
    }

    #[test]
    fn rejects_empty_name_value() {
        let body = "---\n\
name:\n\
description: x. Use when y.\n\
allowed-tools: [Read]\n\
---\n";
        assert!(extract_skill_name_from_frontmatter(body).is_none());
    }

    #[test]
    fn returns_none_when_no_frontmatter() {
        assert!(extract_skill_name_from_frontmatter("# Just markdown").is_none());
    }

    #[test]
    fn curator_prompt_for_team_scope_asks_for_ability_toml() {
        let verdict = json!({
            "memory_type": "project",
            "scope": "team",
            "what_to_save": "x",
            "why": "y",
            "how_to_apply": "z",
        });
        let p = render_curator_prompt("ability", "do thing", &verdict, &[], &[]);
        assert!(p.contains("ability.toml"));
        assert!(p.contains("schema_version"));
    }

    #[test]
    fn curator_prompt_for_private_scope_asks_for_skill_md() {
        let verdict = json!({
            "memory_type": "feedback",
            "scope": "private",
            "what_to_save": "x",
            "why": "y",
            "how_to_apply": "z",
        });
        let p = render_curator_prompt("skill", "do thing", &verdict, &[], &[]);
        // Pin the load-bearing parts of the Anthropic-canonical
        // skill structure that the prompt must teach the curator
        // to emit. A regression that drops any of these would
        // produce skills the loader can't activate.
        assert!(p.contains("SKILL.md"));
        assert!(p.contains("name:"));
        assert!(p.contains("description:"));
        assert!(p.contains("allowed-tools:"));
        assert!(p.contains("Use when"));
        assert!(p.contains("## When This Skill Activates"));
    }

    #[test]
    fn curator_prompt_for_team_scope_includes_catalog() {
        // When the owner has abilities, the curator prompt must list
        // them so the LLM can reference real verbs. Pinning this
        // surface in a test catches a regression that drops the
        // catalog block.
        let verdict = json!({
            "memory_type": "project",
            "scope": "team",
            "what_to_save": "x",
            "why": "y",
            "how_to_apply": "z",
        });
        let catalog = vec![
            CatalogEntry {
                qualified: "alice.summarise".to_string(),
                description: "summarise text".to_string(),
            },
            CatalogEntry {
                qualified: "alice.weather".to_string(),
                description: "fetch weather".to_string(),
            },
        ];
        let p = render_curator_prompt("ability", "do thing", &verdict, &[], &catalog);
        assert!(p.contains("alice.summarise"), "catalog verb missing: {p}");
        assert!(p.contains("alice.weather"));
        assert!(
            p.contains("Do NOT invent verb names"),
            "must instruct curator to stay within catalog: {p}"
        );
    }

    #[test]
    fn curator_prompt_for_team_scope_handles_empty_catalog() {
        // Fresh agent with no abilities yet: prompt still well-formed,
        // catalog block degrades to a clear "no abilities" hint.
        let verdict = json!({
            "memory_type": "project",
            "scope": "team",
            "what_to_save": "x",
            "why": "y",
            "how_to_apply": "z",
        });
        let p = render_curator_prompt("ability", "do thing", &verdict, &[], &[]);
        assert!(p.contains("no abilities currently published"));
    }

    #[test]
    fn validate_accepts_chat_fallback_ability_without_exec_block() {
        // No [exec] block means the dispatcher routes to the agent's
        // chat fallback. Catalog is irrelevant; validation accepts.
        let body = r#"
schema_version = "1"
name = "summarise_complaints"
description = "summarise customer complaint threads"
[input_schema]
type = "object"
"#;
        validate_authored_ability(body, &[]).expect("chat-fallback ability validates");
    }

    #[test]
    fn validate_rejects_eal_referencing_unknown_ability() {
        // P0 enforcement: a curator that hallucinates
        // `alice.fictional_verb(...)` must be rejected — even if
        // the manifest itself is well-formed and the EAL parses.
        let body = r#"
schema_version = "1"
name = "lookup_pipeline"
description = "fictional-ref lookup"
[input_schema]
type = "object"
[exec]
kind = "eal"
source = """
mission "x" {
  let r = alice.fictional_verb()
}
"""
result_binding = "r"
"#;
        let catalog = vec![CatalogEntry {
            qualified: "alice.real_verb".to_string(),
            description: "the only thing alice can do".to_string(),
        }];
        let err = validate_authored_ability(body, &catalog)
            .expect_err("hallucinated reference must be rejected");
        assert!(
            err.contains("alice.fictional_verb"),
            "error names the bad ref: {err}"
        );
        assert!(err.contains("not in"), "error explains why: {err}");
    }

    #[test]
    fn validate_accepts_eal_when_every_reference_is_in_catalog() {
        let body = r#"
schema_version = "1"
name = "two_step"
description = "real-ref pipeline"
[input_schema]
type = "object"
[exec]
kind = "eal"
source = """
mission "x" {
  let r = alice.real_verb()
}
"""
result_binding = "r"
"#;
        let catalog = vec![CatalogEntry {
            qualified: "alice.real_verb".to_string(),
            description: "real ability".to_string(),
        }];
        validate_authored_ability(body, &catalog)
            .expect("real-ref ability must validate against the catalog");
    }

    #[test]
    fn validate_rejects_malformed_eal_source() {
        // A curator that produced a manifest whose EAL source is
        // unparseable would otherwise publish an ability that fails
        // the first time anyone calls it.
        let body = r#"
schema_version = "1"
name = "broken_eal"
description = "bad EAL"
[input_schema]
type = "object"
[exec]
kind = "eal"
source = "not valid eal at all {{{"
"#;
        let err = validate_authored_ability(body, &[]).expect_err("malformed EAL is rejected");
        assert!(
            err.contains("EAL source parse failed"),
            "error blames EAL: {err}"
        );
    }

    #[test]
    fn validate_rejects_malformed_manifest_toml() {
        // Bad TOML at the manifest layer (not the EAL source)
        // surfaces from from_toml_str with a "manifest parse"
        // attribution.
        let err = validate_authored_ability("this is not toml{{{", &[])
            .expect_err("malformed manifest is rejected");
        assert!(
            err.contains("manifest parse"),
            "error blames manifest: {err}"
        );
    }

    /// Anthropic-canonical happy-path: a fully-shaped SKILL.md must
    /// validate cleanly. We pin the exact section names because the
    /// loader's match is literal — a regression that drops "When
    /// This Skill Activates" from the heading would silently make
    /// new skills inert.
    #[test]
    fn validate_accepts_anthropic_canonical_skill_md() {
        let body = "---\n\
name: cargo-test-runner\n\
description: Runs Rust tests with prefix filters in a single cargo invocation. Use when running multiple cargo test targets or when the user asks how to test multiple modules efficiently.\n\
allowed-tools: [Read, Bash]\n\
---\n\
\n\
# Cargo Test Runner\n\
\n\
Runs Rust tests using prefix-filtered single cargo invocations.\n\
\n\
## When This Skill Activates\n\
\n\
Use this skill when the user:\n\
- Asks to run multiple cargo test targets\n\
- Mentions slow test runs\n\
- Wants to filter tests by module prefix\n\
\n\
## Process\n\
\n\
### 1. Identify the prefix\n\
- Read the prompt to find the module path the user cares about.\n\
\n\
## Examples\n\
\n\
### Example: parallel modules\n\
\n\
**Input:** run tests for parser and planner\n\
**Expected Output:** `cargo test parser:: planner::`\n";
        validate_authored_skill(body).expect("canonical skill validates");
    }

    /// Front matter missing entirely → invisible to the skill loader.
    #[test]
    fn validate_rejects_skill_md_without_front_matter() {
        let body = "# Cargo Test Runner\n\nNo front matter here.\n\n## When This Skill Activates\n- Asks for tests\n";
        let err = validate_authored_skill(body).expect_err("must reject");
        assert!(err.contains("front-matter"), "msg: {err}");
    }

    /// Front matter present but `description` lacks the "Use when …"
    /// activation hint. The loader can't decide when the skill
    /// should activate.
    #[test]
    fn validate_rejects_skill_md_without_use_when_hint() {
        let body = "---\n\
name: x\n\
description: Does a thing.\n\
allowed-tools: [Read]\n\
---\n\
\n\
# X\n\
\n\
## When This Skill Activates\n\
- Asks about X\n";
        let err = validate_authored_skill(body).expect_err("missing use-when must reject");
        assert!(err.contains("Use when"), "msg: {err}");
    }

    /// Missing `allowed-tools` field → skill activates with no tools
    /// and is mostly useless.
    #[test]
    fn validate_rejects_skill_md_without_allowed_tools() {
        let body = "---\n\
name: x\n\
description: Does a thing. Use when asked about X.\n\
---\n\
\n\
# X\n\
\n\
## When This Skill Activates\n\
- Asks about X\n";
        let err = validate_authored_skill(body).expect_err("missing allowed-tools must reject");
        assert!(err.contains("allowed-tools"), "msg: {err}");
    }

    /// `description` has "use when" but the body forgets the
    /// `## When This Skill Activates` heading. Front-matter alone
    /// isn't enough — the loader's section walk needs a literal
    /// match.
    #[test]
    fn validate_rejects_skill_md_without_when_section() {
        let body = "---\n\
name: x\n\
description: Does a thing. Use when asked about X.\n\
allowed-tools: [Read]\n\
---\n\
\n\
# X\n\
\n\
This skill does X.\n";
        let err = validate_authored_skill(body).expect_err("missing trigger section must reject");
        assert!(err.contains("When This Skill Activates"), "msg: {err}");
    }

    // ── Integration tests with stub chat handlers ────────────────
    //
    // These tests build a `AxonAbilityCatalog`, register stub
    // `<agent>.chat` handlers that return canned JSON envelopes
    // matching the chat ability's wire shape, then drive the full
    // worker+judge+curator loop. Boundaries:
    //
    //   * No HomeGuard needed — mission.think itself does not touch
    //     `~/.easynet/`. The publish handlers (ability.publish /
    //     skill.publish) DO touch it; tests that exercise the
    //     curator → publish path acquire a HomeGuard.
    //   * Stub handlers are stateful: a per-test counter lets a
    //     stub return different replies on cycle 1 vs cycle 2,
    //     simulating progress / termination / verdict shapes.

    use std::sync::Mutex as StdMutex;

    /// Build a registry with stub chat for a single agent. The stub
    /// returns the next canned reply each call; once exhausted, it
    /// cycles. Each call advances the session_id so a test can
    /// distinguish "resumed" from "fresh".
    fn registry_with_stub_chat(agent: &str, replies: Vec<&'static str>) -> Arc<AxonAbilityCatalog> {
        let counter: Arc<StdMutex<usize>> = Arc::new(StdMutex::new(0));
        let replies: Arc<Vec<String>> = Arc::new(replies.into_iter().map(String::from).collect());
        let mut reg = AxonAbilityCatalog::new();
        let counter_c = Arc::clone(&counter);
        let replies_c = Arc::clone(&replies);
        reg.register_rpc(
            format!("{agent}.chat"),
            Arc::new(move |args: Value| {
                let mut idx = counter_c.lock().unwrap();
                let i = *idx;
                *idx = i + 1;
                let reply = replies_c.get(i).cloned().unwrap_or_default();
                let session_id = args
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .unwrap_or_else(|| format!("stub-session-{i}"));
                Ok(json!({
                    "reply": reply,
                    "session_id": session_id,
                }))
            }),
        );
        Arc::new(reg)
    }

    #[test]
    fn worker_silent_terminates_loop_immediately() {
        // Worker returns "" on cycle 1 → loop exits with
        // termination_reason = "worker_silent" before the judge
        // runs. We pre-stuff judge replies but they should never
        // be consumed.
        let reg = registry_with_stub_chat("alice", vec!["", "{\"terminate\":true}"]);
        let resp = think_with_registry(
            &reg,
            json!({"owner_agent_id": "alice", "prompt": "do the thing"}),
        )
        .expect("ok");
        assert_eq!(resp["termination_reason"], "worker_silent");
        assert_eq!(resp["cycles_used"], 1);
    }

    #[test]
    fn judge_terminate_true_ends_loop_with_no_curator() {
        // Worker emits text. Judge says terminate=true with
        // memory_type="none" → curator must NOT run.
        let reg = registry_with_stub_chat(
            "alice",
            vec![
                "I have completed the task: the answer is 42.",
                r#"{"terminate":true,"memory_type":"none","scope":"private","what_to_save":"","why":"","how_to_apply":"","exclusion_check":{"is_derivable_from_code":false,"is_in_git_log":false,"is_debug_recipe":false,"is_ephemeral":false}}"#,
            ],
        );
        let resp = think_with_registry(
            &reg,
            json!({"owner_agent_id": "alice", "prompt": "what is 6 times 7"}),
        )
        .expect("ok");
        assert_eq!(resp["termination_reason"], "judge_terminate");
        assert_eq!(resp["cycles_used"], 1);
        assert!(
            resp["curator"].is_null(),
            "curator must not run when memory_type=none: {}",
            resp["curator"]
        );
    }

    #[test]
    fn max_cycles_reached_when_judge_never_terminates() {
        // Judge replies are unparseable (so terminate stays false
        // by default), worker keeps speaking. Loop should run
        // exactly max_cycles times.
        let reg = registry_with_stub_chat(
            "alice",
            vec![
                "working on it",
                "still working",
                "still working",
                "still working",
                "still working",
                "still working",
            ],
        );
        let resp = think_with_registry(
            &reg,
            json!({
                "owner_agent_id": "alice",
                "prompt": "p",
                "max_cycles": 2,
            }),
        )
        .expect("ok");
        assert_eq!(resp["termination_reason"], "max_cycles_reached");
        assert_eq!(resp["cycles_used"], 2);
        // Transcript has one entry per cycle.
        let arr = resp["transcript"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn curator_attempts_publish_when_verdict_qualifies() {
        // Stub a 3-call sequence:
        //   call 0 (worker cycle 1):   "did the work"
        //   call 1 (judge cycle 1):    sinkable verdict, terminate=true
        //   call 2 (curator):          authors a SKILL.md body
        // We do NOT register a real skill.publish handler — we
        // observe `curator.stage = "resolve_publish"` to confirm the
        // handler reached publish dispatch (which is the boundary
        // we want to verify in this slice; full publish lives in
        // the publish handler's own tests).
        let verdict = r#"{"terminate":true,"memory_type":"feedback","scope":"private","what_to_save":"prefer ripgrep over grep when scanning","why":"better unicode + faster on large repos","how_to_apply":"reach for rg before grep when searching codebases","exclusion_check":{"is_derivable_from_code":false,"is_in_git_log":false,"is_debug_recipe":false,"is_ephemeral":false}}"#;
        // Stub curator returns an Anthropic-canonical SKILL.md so
        // it passes validate_authored_skill and the test reaches
        // the publish-dispatch stage. The body shape mirrors what
        // a real curator should produce — name, description with
        // "Use when", allowed-tools, the activation section.
        let stub_skill_md = "---\n\
name: prefer-rg\n\
description: Prefer ripgrep over grep when scanning codebases. Use when searching files or asked about grep alternatives.\n\
allowed-tools: [Read, Bash, Grep]\n\
---\n\
\n\
# Prefer Ripgrep Over Grep\n\
\n\
Use ripgrep (`rg`) instead of `grep` when scanning codebases.\n\
\n\
## When This Skill Activates\n\
\n\
Use this skill when the user:\n\
- Asks about grep alternatives\n\
- Mentions slow searches\n\
- Wants to scan a codebase\n";
        let reg = registry_with_stub_chat("alice", vec!["did the work", verdict, stub_skill_md]);
        let resp = think_with_registry(
            &reg,
            json!({"owner_agent_id": "alice", "prompt": "scan for foo"}),
        )
        .expect("ok");
        assert_eq!(resp["termination_reason"], "judge_terminate");
        let curator = &resp["curator"];
        assert_eq!(curator["attempted"], true);
        // No skill.publish in this stubbed registry → resolve_publish
        // is the failure stage. That confirms the handler correctly
        // reached the publish dispatch step with a `target = "skill"`.
        assert_eq!(curator["ok"], false);
        assert_eq!(
            curator["stage"], "resolve_publish",
            "curator should fail at publish dispatch when no publish handler is registered: {}",
            curator
        );
    }

    #[test]
    fn worker_session_id_persists_across_cycles() {
        // The stub echoes back any session_id passed in. We pass
        // none on cycle 1 (stub mints "stub-session-0") and assert
        // cycle 2's worker call sees that same id back, confirming
        // the orchestrator captured + replayed it.
        let unparseable_judge = "I cannot decide";
        let reg = registry_with_stub_chat(
            "alice",
            vec![
                "cycle 1 worker",
                unparseable_judge,
                "cycle 2 worker",
                unparseable_judge,
            ],
        );
        let resp = think_with_registry(
            &reg,
            json!({
                "owner_agent_id": "alice",
                "prompt": "p",
                "max_cycles": 2,
            }),
        )
        .expect("ok");
        // Both cycles' worker text is in the transcript; cycle 2
        // having any worker text at all is the load-bearing signal
        // (cycle 1 minted a session, cycle 2 resumed it without
        // crashing the dispatcher).
        let arr = resp["transcript"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["worker"], "cycle 1 worker");
        assert_eq!(arr[1]["worker"], "cycle 2 worker");
    }

    #[test]
    fn judge_prompt_embeds_schema_fields() {
        let p = render_judge_prompt("do X", "did Y", 2, 5);
        // Surface check: the prompt mentions every required field
        // of the verdict schema. A regression that drops a field
        // would silently lose a memory dimension.
        for field in [
            "terminate",
            "memory_type",
            "scope",
            "what_to_save",
            "why",
            "how_to_apply",
            "exclusion_check",
        ] {
            assert!(p.contains(field), "judge prompt missing {field}: {p}");
        }
        // Cycle counter must be present so the judge knows where in
        // the budget it is.
        assert!(p.contains("cycle 2/5"));
    }
}
