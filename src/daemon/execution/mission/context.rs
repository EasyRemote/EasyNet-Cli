// EasyNet CLI — Dispatch Context
// ===============================
//
// File: src/agent/context.rs
// Description: Typed, thread-local context for cross-agent dispatch.
//
// Why this module exists
// ----------------------
// Every cross-agent dispatch in EasyNet must originate from a *mission
// context*: a typed bundle carrying the active mission run id, the
// recursion depth, and (eventually) the tenant + origin-agent identity.
// For the first several iterations of this codebase that bundle was
// expressed as two process-global env vars — `EASYNET_MISSION_ID` and
// `EASYNET_AGENT_DEPTH` — which had three problems:
//
//   1. Stringly typed. Every read site re-parsed the depth integer and
//      hand-checked the mission-id format. A typo in any of the ~6 read
//      sites silently produced wrong audit data.
//   2. Process-global. The moment the runtime grows in-process parallel
//      mission execution, two missions on different threads stomp each
//      other's env vars.
//   3. Hidden control-flow. A reader couldn't tell from a function
//      signature whether it depended on the env vars; the contract was
//      only visible by grepping for the magic strings.
//
// This module is the typed in-process channel. The env vars survive at
// exactly one boundary — when we spawn an external agent CLI as a
// subprocess (claude-code, codex). At that boundary the child *does*
// need to learn its parent's depth and mission id, and the subprocess
// environment is the only mechanism that crosses the process line.
//
// Lifetime model
// --------------
// Context is set via `with_context(ctx, || { ... })`, which installs `ctx`
// as the current thread's active context for the duration of the closure
// and restores the previous value on exit (panic-safe via Drop). Code
// inside the closure reads `current()` to obtain the typed bundle, never
// touching env vars directly.
//
// At the cross-process boundary, `serialize_to_env` writes the active
// context into a `BTreeMap` of env vars that gets handed to the child.
// At the child's entry point (a fresh process), `from_env` parses the
// env vars back into a `DispatchContext` so the child has the same
// typed view its parent had.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Reserved env-var keys at the subprocess boundary. Kept here, not in
/// dispatch.rs, so any future read/write site goes through this module.
const ENV_MISSION_ID: &str = "EASYNET_MISSION_ID";
const ENV_AGENT_DEPTH: &str = "EASYNET_AGENT_DEPTH";
const ENV_ORIGIN_AGENT: &str = "EASYNET_ORIGIN_AGENT";

/// Typed mission context for one cross-agent dispatch chain.
///
/// Cheap to clone — all fields are owned strings/paths/integers, and a
/// dispatch chain typically has one context per process so the clone
/// count is bounded by the recursion depth (≤ MAX_AGENT_DEPTH).
#[derive(Debug, Clone)]
pub struct DispatchContext {
    /// Identifier of the mission run that originated this dispatch. The
    /// id is the directory name of a real run dir under
    /// `~/.easynet/missions/runs/<id>/`; that fact is checked at dispatch
    /// time as a basic anti-forgery measure.
    pub mission_id: String,

    /// Recursion depth. Incremented whenever a context is propagated to
    /// a child agent. The dispatch guard refuses to spawn another agent
    /// once `depth >= MAX_AGENT_DEPTH` (defined in `dispatch.rs`).
    pub depth: u32,

    /// Optional path to the mission run directory. Set when the runner
    /// has a stable on-disk record; absent for synthetic contexts (e.g.
    /// the dispatch tests, which only need the depth check).
    pub mission_run_dir: Option<PathBuf>,

    /// Optional originating agent name (e.g. `"claude"`). Surfaced in
    /// the audit log so operators can attribute a dispatch chain to the
    /// agent that started it.
    pub origin_agent: Option<String>,
}

impl DispatchContext {
    /// Construct a context for a freshly-started mission (depth = 0).
    pub fn for_mission(mission_id: impl Into<String>, mission_run_dir: PathBuf) -> Self {
        Self {
            mission_id: mission_id.into(),
            depth: 0,
            mission_run_dir: Some(mission_run_dir),
            origin_agent: None,
        }
    }

    /// Derive a child context for the next link in the dispatch chain.
    ///
    /// The depth is incremented once (saturating at `u32::MAX`, which the
    /// depth guard in `dispatch.rs` rejects long before we could hit).
    ///
    /// `caller` is the name of the agent currently performing the
    /// dispatch — *not* necessarily the origin. The origin of a chain is
    /// defined as "the first agent to dispatch"; once seeded it is
    /// immutable. So:
    ///
    /// - If `self.origin_agent` is `None` (this is the root mission
    ///   runtime dispatching the first link), seed the origin with
    ///   `caller`.
    /// - Otherwise preserve `self.origin_agent` verbatim — every
    ///   subsequent child in the chain reports the same root.
    ///
    /// This invariant is what makes audit logs attributable: given any
    /// node in the dispatch tree, the origin tells an operator which
    /// human-facing agent started the chain.
    pub fn child(&self, caller: impl Into<String>) -> Self {
        let origin_agent = self.origin_agent.clone().or_else(|| Some(caller.into()));
        Self {
            mission_id: self.mission_id.clone(),
            depth: self.depth.saturating_add(1),
            mission_run_dir: self.mission_run_dir.clone(),
            origin_agent,
        }
    }

    /// Write the context into an env-var map suitable for handing to a
    /// `Command::env()` call when spawning a subprocess. This is the
    /// *only* sanctioned place where the context crosses a process line.
    ///
    /// Every non-empty field is emitted. `origin_agent` is only written
    /// when set, because a leading root mission (with no agent yet
    /// dispatched) has no origin to propagate and an empty value would
    /// round-trip to `Some("")` on the receiving side, which is worse
    /// than the field being absent.
    pub fn serialize_to_env(&self, env: &mut BTreeMap<String, String>) {
        env.insert(ENV_MISSION_ID.to_string(), self.mission_id.clone());
        env.insert(ENV_AGENT_DEPTH.to_string(), self.depth.to_string());
        if let Some(origin) = &self.origin_agent {
            if !origin.is_empty() {
                env.insert(ENV_ORIGIN_AGENT.to_string(), origin.clone());
            }
        }
    }

    /// Recover a context from the process env vars. Used at the entry
    /// point of a *fresh process* — never inside the same process where
    /// `enter` is the right channel.
    ///
    /// Returns `None` if the env vars are missing or malformed; callers
    /// decide whether absence is an error (production) or expected
    /// (development / tests).
    pub fn from_env() -> Option<Self> {
        let mission_id = std::env::var(ENV_MISSION_ID).ok()?;
        if mission_id.is_empty() {
            return None;
        }
        let depth = std::env::var(ENV_AGENT_DEPTH).ok()?.parse::<u32>().ok()?;
        let origin_agent = std::env::var(ENV_ORIGIN_AGENT)
            .ok()
            .filter(|s| !s.is_empty());
        Some(Self {
            mission_id,
            depth,
            mission_run_dir: None,
            origin_agent,
        })
    }
}

thread_local! {
    /// The active dispatch context for this thread, if any.
    /// `RefCell` is sound here because the cell is only ever borrowed for
    /// the brief reads/writes inside `with_context` / `current` — no
    /// borrow ever spans an `await` (this codebase is synchronous) or a
    /// reentrant call.
    static CURRENT: RefCell<Option<DispatchContext>> = const { RefCell::new(None) };
}

/// RAII scope guard for the thread-local context. Constructed by
/// `enter` (Drop-based) or `with_context` (closure-based).
///
/// On Drop the previous thread-local value (which may be `None`) is
/// restored, so nested guards behave like a stack and a guard outliving
/// its scope cannot leak the installed context to subsequent unrelated
/// code on the same thread.
#[must_use = "DispatchContext only stays installed while the Guard is alive"]
pub struct ContextGuard {
    prev: Option<DispatchContext>,
}

impl Drop for ContextGuard {
    fn drop(&mut self) {
        CURRENT.with(|cell| *cell.borrow_mut() = self.prev.take());
    }
}

/// Install `ctx` as the current thread's dispatch context, returning a
/// guard whose `Drop` restores the previous value.
///
/// This is the only public installer — the mission runs helper holds
/// the returned guard as a struct field, and tests construct one in a
/// scope. A closure-based wrapper (`with_context(ctx, || …)`) was
/// considered, but every existing caller is happier with the explicit
/// guard, and adding a wrapper that no production path uses just to
/// have a "nice" surface trips the dead-code lint.
pub fn enter(ctx: DispatchContext) -> ContextGuard {
    let prev = CURRENT.with(|cell| cell.borrow_mut().replace(ctx));
    ContextGuard { prev }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DispatchContextSource {
    ThreadLocal,
    ProcessEnvironment,
}

#[derive(Debug, Clone)]
struct ResolvedDispatchContext {
    source: DispatchContextSource,
    context: DispatchContext,
}

fn current_with_source() -> Option<ResolvedDispatchContext> {
    if let Some(context) = CURRENT.with(|cell| cell.borrow().clone()) {
        return Some(ResolvedDispatchContext {
            source: DispatchContextSource::ThreadLocal,
            context,
        });
    }

    DispatchContext::from_env().map(|context| ResolvedDispatchContext {
        source: DispatchContextSource::ProcessEnvironment,
        context,
    })
}

/// Read the current dispatch context from the explicit source chain.
///
/// The source ordering is load-bearing: in-process mission execution
/// reads the thread-local context installed by `enter`; child agent
/// subprocesses start without that thread-local and reconstruct the same
/// typed context from the serialized process-environment handoff. This
/// preserves recursion depth and audit attribution across process
/// boundaries without treating the environment as a degraded compatibility
/// path.
pub fn current() -> Option<DispatchContext> {
    let resolved = current_with_source()?;
    match resolved.source {
        DispatchContextSource::ThreadLocal | DispatchContextSource::ProcessEnvironment => {
            Some(resolved.context)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn ctx(mission_id: &str, depth: u32) -> DispatchContext {
        DispatchContext {
            mission_id: mission_id.to_string(),
            depth,
            mission_run_dir: None,
            origin_agent: None,
        }
    }

    #[test]
    fn enter_installs_and_restores_on_drop() {
        // Outside the guard scope, no context is set.
        assert!(CURRENT.with(|c| c.borrow().is_none()));
        {
            let _g = enter(ctx("m1", 0));
            assert_eq!(
                CURRENT.with(|c| c.borrow().as_ref().map(|x| x.mission_id.clone())),
                Some("m1".to_string())
            );
        }
        // After the guard drops, the previous None is restored.
        assert!(CURRENT.with(|c| c.borrow().is_none()));
    }

    #[test]
    fn nested_guards_act_like_a_stack() {
        let _outer = enter(ctx("outer", 0));
        {
            let _inner = enter(ctx("inner", 1));
            let cur = CURRENT.with(|c| c.borrow().clone()).unwrap();
            assert_eq!(cur.mission_id, "inner");
            assert_eq!(cur.depth, 1);
        }
        // Inner guard dropped — outer must be restored.
        let cur = CURRENT.with(|c| c.borrow().clone()).unwrap();
        assert_eq!(cur.mission_id, "outer");
    }

    #[test]
    fn restores_on_panic() {
        // The restore is a Drop guard, so a panic inside the scope must
        // still leave the thread-local clean for subsequent tests.
        let outcome = std::panic::catch_unwind(|| {
            let _g = enter(ctx("doomed", 0));
            panic!("boom");
        });
        assert!(outcome.is_err());
        assert!(CURRENT.with(|c| c.borrow().is_none()));
    }

    #[test]
    fn child_seeds_origin_when_parent_has_none() {
        // Root mission dispatches its first agent — the caller's name
        // becomes the origin.
        let parent = ctx("m", 0);
        let child = parent.child("claude");
        assert_eq!(child.mission_id, parent.mission_id);
        assert_eq!(child.depth, 1);
        assert_eq!(child.origin_agent.as_deref(), Some("claude"));
    }

    #[test]
    fn child_preserves_origin_across_chain() {
        // The origin is seeded once and must propagate verbatim through
        // every subsequent link. A→B→C all report origin = A.
        let root = ctx("m", 0);
        let a = root.child("claude");
        let b = a.child("codex");
        let c = b.child("codex-web");
        assert_eq!(c.origin_agent.as_deref(), Some("claude"));
        assert_eq!(c.depth, 3);
    }

    #[test]
    fn serialize_round_trip_preserves_origin() {
        // The env-var channel is the wire format; a serialize→deserialize
        // cycle must recover mission id, depth, and origin. This pins
        // cross-process fidelity of the audit tuple.
        let mut env = BTreeMap::new();
        let parent = DispatchContext {
            mission_id: "mission-42".to_string(),
            depth: 3,
            mission_run_dir: None,
            origin_agent: Some("claude".to_string()),
        };
        parent.serialize_to_env(&mut env);
        assert_eq!(
            env.get(ENV_MISSION_ID).map(String::as_str),
            Some("mission-42")
        );
        assert_eq!(env.get(ENV_AGENT_DEPTH).map(String::as_str), Some("3"));
        assert_eq!(
            env.get(ENV_ORIGIN_AGENT).map(String::as_str),
            Some("claude")
        );

        // Mirror serialize_to_env → process env → from_env.
        // We do the env mutation under a mutex because std::env::set_var
        // is process-global; multiple tests touching env vars at once
        // would alias. The mutex is a crate-private test helper.
        let _lock = test_env_lock().lock().unwrap();
        std::env::set_var(ENV_MISSION_ID, env.get(ENV_MISSION_ID).unwrap());
        std::env::set_var(ENV_AGENT_DEPTH, env.get(ENV_AGENT_DEPTH).unwrap());
        std::env::set_var(ENV_ORIGIN_AGENT, env.get(ENV_ORIGIN_AGENT).unwrap());
        let recovered = DispatchContext::from_env().expect("present");
        std::env::remove_var(ENV_MISSION_ID);
        std::env::remove_var(ENV_AGENT_DEPTH);
        std::env::remove_var(ENV_ORIGIN_AGENT);
        assert_eq!(recovered.mission_id, "mission-42");
        assert_eq!(recovered.depth, 3);
        assert_eq!(recovered.origin_agent.as_deref(), Some("claude"));
    }

    #[test]
    fn current_reports_process_environment_source_for_child_handoff() {
        let _lock = test_env_lock().lock().unwrap();
        std::env::set_var(ENV_MISSION_ID, "child-process-run");
        std::env::set_var(ENV_AGENT_DEPTH, "4");
        std::env::remove_var(ENV_ORIGIN_AGENT);

        let resolved = current_with_source().expect("process environment handoff");

        std::env::remove_var(ENV_MISSION_ID);
        std::env::remove_var(ENV_AGENT_DEPTH);
        std::env::remove_var(ENV_ORIGIN_AGENT);

        assert_eq!(resolved.source, DispatchContextSource::ProcessEnvironment);
        assert_eq!(resolved.context.mission_id, "child-process-run");
        assert_eq!(resolved.context.depth, 4);
    }

    #[test]
    fn current_prefers_thread_local_source_over_process_environment() {
        let _lock = test_env_lock().lock().unwrap();
        std::env::set_var(ENV_MISSION_ID, "env-run");
        std::env::set_var(ENV_AGENT_DEPTH, "9");
        let _guard = enter(ctx("thread-run", 2));

        let resolved = current_with_source().expect("thread-local context");

        std::env::remove_var(ENV_MISSION_ID);
        std::env::remove_var(ENV_AGENT_DEPTH);
        std::env::remove_var(ENV_ORIGIN_AGENT);

        assert_eq!(resolved.source, DispatchContextSource::ThreadLocal);
        assert_eq!(resolved.context.mission_id, "thread-run");
        assert_eq!(resolved.context.depth, 2);
    }

    #[test]
    fn process_environment_handoff_rejects_missing_or_malformed_depth() {
        let _lock = test_env_lock().lock().unwrap();

        std::env::set_var(ENV_MISSION_ID, "missing-depth");
        std::env::remove_var(ENV_AGENT_DEPTH);
        assert!(DispatchContext::from_env().is_none());

        std::env::set_var(ENV_AGENT_DEPTH, "not-a-depth");
        assert!(DispatchContext::from_env().is_none());

        std::env::remove_var(ENV_MISSION_ID);
        std::env::remove_var(ENV_AGENT_DEPTH);
        std::env::remove_var(ENV_ORIGIN_AGENT);
    }

    #[test]
    fn serialize_omits_origin_when_absent() {
        // Root mission has no origin yet — don't emit an empty env var,
        // because from_env would parse it as Some("") and downstream
        // audit lines would print an empty agent name.
        let mut env = BTreeMap::new();
        let root = ctx("m", 0);
        root.serialize_to_env(&mut env);
        assert!(!env.contains_key(ENV_ORIGIN_AGENT));
    }

    /// Per-process mutex for tests that mutate `std::env` — serialises
    /// access to the process-global env table so concurrent test cases
    /// don't stomp each other.
    fn test_env_lock() -> &'static std::sync::Mutex<()> {
        use std::sync::OnceLock;
        static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    #[test]
    fn child_threads_do_not_see_parent_thread_local() {
        // Thread-locals are per-thread by definition; this test pins
        // that property so a future "let's use a global RefCell" refactor
        // (which would re-introduce the cross-thread race the env-var
        // approach already had) fails loudly. We synchronise the parent
        // and child via a Barrier so the child observes the parent's
        // state *while it is installed*, not before or after.
        let barrier = Arc::new(Barrier::new(2));
        let b2 = Arc::clone(&barrier);
        let child = std::thread::spawn(move || {
            b2.wait();
            CURRENT.with(|c| c.borrow().is_none())
        });
        let _g = enter(ctx("parent-only", 0));
        barrier.wait();
        let child_saw_none = child.join().unwrap();
        assert!(
            child_saw_none,
            "child thread must NOT see the parent thread's installed context"
        );
    }
}
