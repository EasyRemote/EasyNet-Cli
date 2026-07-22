// EasyNet CLI — Async/Sync bridge helpers
// =======================================
//
// File: src/support/async_bridge.rs
//
// Centralised "run a future to completion from sync code" recipe.
// Before this module, three near-identical implementations lived in
// `daemon/ability/dispatch.rs` (`block_on_runtime_sync`),
// the agent lifecycle system ability (`block_on_hot_registrar`),
// and `daemon/invocation/local_runtime_invoker.rs` (`block_on_runtime`). They
// disagreed on what to do when called from outside a tokio runtime —
// `block_on_runtime_sync` fell back to `futures::executor::block_on`,
// `block_on_hot_registrar` returned `None`, and `block_on_runtime`
// built a fresh current-thread runtime.
//
// Industrial-textbook reason to unify: the three differed only in
// fallback policy. Putting the policy in an enum and the dispatch in
// one helper means the next caller that wants this recipe doesn't
// invent a fourth slightly-different shape.
//
// Why this lives in `support/` not `runtime/`
// -------------------------------------------
// `support/` is the leaf-layer for cross-cutting plumbing per
// `src/support/mod.rs`. The async/sync bridge is exactly that:
// every layer that hosts a sync handler (runtime registry, hot
// registrar, CLI subcommands that dispatch through the local Axon
// LocalRuntime) reaches for it. Pinning the recipe here, with no
// runtime-specific imports, keeps the call sites cheap.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::future::Future;

/// What to do when the caller is NOT inside a tokio runtime. The
/// three production sites pre-unification each picked a different
/// answer; the enum makes the choice visible at the call site
/// rather than buried inside three separately-evolving helpers.
#[derive(Debug, Clone, Copy)]
pub enum NoRuntimeFallback {
    /// Drive the future on the calling thread via
    /// `futures::executor::block_on`. Cheapest; safe when the
    /// future does no I/O that requires a tokio reactor (handler
    /// closures that only touch sync state). The original
    /// `block_on_runtime_sync` recipe.
    UseFuturesExecutor,
    /// Build a fresh current-thread tokio runtime via
    /// `tokio::runtime::Builder::new_current_thread().enable_all()`,
    /// drop it after `block_on` returns. Necessary when the future
    /// awaits a tokio resource (timers, sockets) outside a hosting
    /// runtime — e.g. CLI bridge code reaching into a
    /// LocalRuntime from a non-tokio entry point. The original
    /// `local_runtime_invoker::block_on_runtime` recipe.
    BuildCurrentThreadTokio,
}

/// Run `future` to completion from sync code, with explicit
/// fallback policy. Inside a multi-threaded tokio runtime, defers
/// to `block_in_place` so the current worker isn't pinned. Inside a
/// current-thread tokio runtime, applies `fallback`: callers that
/// only need in-memory futures can keep the cheap futures executor,
/// while callers that await tokio resources get a fresh tokio runtime
/// on a separate scoped thread. Outside tokio entirely, applies the
/// same explicit fallback policy on the calling thread.
pub fn run_blocking<F>(future: F, fallback: NoRuntimeFallback) -> F::Output
where
    F: Future,
    F::Output: Send,
    F: Send,
{
    try_run_blocking(
        future,
        fallback,
        "build current-thread tokio runtime for sync bridge",
    )
    .expect("sync bridge runtime construction failed")
}

/// Fallible variant of [`run_blocking`] for boot/product surfaces
/// that must report a typed skip event instead of panicking when the
/// helper runtime cannot be constructed.
///
/// `bridge_label` is included in the error string so the operator can
/// identify the exact bridge call site from logs without stack traces.
pub fn try_run_blocking<F>(
    future: F,
    fallback: NoRuntimeFallback,
    bridge_label: &str,
) -> Result<F::Output, String>
where
    F: Future,
    F::Output: Send,
    F: Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle)
            if matches!(
                handle.runtime_flavor(),
                tokio::runtime::RuntimeFlavor::MultiThread
            ) =>
        {
            Ok(tokio::task::block_in_place(|| handle.block_on(future)))
        }
        // Single-thread tokio context. `handle.block_on(future)` is
        // illegal — tokio rejects re-entering its own runtime. Honor
        // the call site's explicit policy instead of guessing.
        Ok(_) => match fallback {
            NoRuntimeFallback::UseFuturesExecutor => Ok(futures::executor::block_on(future)),
            NoRuntimeFallback::BuildCurrentThreadTokio => {
                block_on_fresh_current_thread_tokio_on_thread(future)
                    .map_err(|error| format!("{bridge_label}: {error}"))
            }
        },
        Err(_) => match fallback {
            NoRuntimeFallback::UseFuturesExecutor => Ok(futures::executor::block_on(future)),
            NoRuntimeFallback::BuildCurrentThreadTokio => {
                block_on_fresh_current_thread_tokio(future)
                    .map_err(|error| format!("{bridge_label}: {error}"))
            }
        },
    }
}

fn block_on_fresh_current_thread_tokio<F>(future: F) -> Result<F::Output, std::io::Error>
where
    F: Future,
{
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(future))
}

fn block_on_fresh_current_thread_tokio_on_thread<F>(future: F) -> Result<F::Output, std::io::Error>
where
    F: Future + Send,
    F::Output: Send,
{
    std::thread::scope(|scope| {
        scope
            .spawn(move || block_on_fresh_current_thread_tokio(future))
            .join()
            .expect("sync bridge helper thread panicked")
    })
}

/// Try to drive `future` from sync code when a tokio runtime is
/// already present, returning `None` if there is no tokio handle at
/// all. Used by call sites that explicitly want the "no tokio -> skip"
/// semantics rather than either no-runtime fallback in
/// [`run_blocking`].
///
/// Current-thread tokio runtimes cannot be re-entered, and
/// `futures::executor::block_on` is not a valid substitute for futures
/// that need tokio's reactor. Match [`run_blocking`]'s
/// `BuildCurrentThreadTokio` path and offload to a fresh helper-thread
/// runtime instead.
pub fn try_run_blocking_in_tokio<F>(future: F) -> Option<F::Output>
where
    F: Future,
    F::Output: Send,
    F: Send,
{
    let handle = tokio::runtime::Handle::try_current().ok()?;
    if matches!(
        handle.runtime_flavor(),
        tokio::runtime::RuntimeFlavor::MultiThread
    ) {
        Some(tokio::task::block_in_place(|| handle.block_on(future)))
    } else {
        block_on_fresh_current_thread_tokio_on_thread(future).ok()
    }
}

/// Spawn a named detached thread that hosts one current-thread tokio
/// runtime and drives `future` to completion.
///
/// This is the canonical lifecycle boundary for sync boot paths that
/// need a background async worker but cannot require an ambient tokio
/// runtime. Runtime-construction failure is reported through
/// `on_runtime_build_failed`; thread-spawn failure is returned to the
/// caller so the domain surface can log the right lifecycle event.
pub fn spawn_current_thread_tokio<F, E>(
    name: impl Into<String>,
    future: F,
    on_runtime_build_failed: E,
) -> Result<(), std::io::Error>
where
    F: Future<Output = ()> + Send + 'static,
    E: FnOnce(std::io::Error) + Send + 'static,
{
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || match block_on_fresh_current_thread_tokio(future) {
            Ok(()) => {}
            Err(error) => on_runtime_build_failed(error),
        })
        .map(|_| ())
}

/// Discard a `tokio::sync::mpsc::Sender::try_send` result with
/// structured classification. `try_send` distinguishes "channel
/// closed" (receiver dropped — legitimate end-of-life) from
/// "channel full" (backpressure pushback — a real failure mode
/// for the EOF/terminal frames the daemon emits). The bare
/// `let _ = sender.try_send(...)` shape buries that distinction.
///
/// Emits `kind = try_send_dropped_full` op_event when the failure
/// is backpressure (which means a terminal frame was lost and the
/// receiver may hang waiting for it). Closed-receiver failures are
/// silent — that path is expected on client disconnect.
///
/// `component` and `kind` annotate the log line so SRE can grep by
/// emit-site without unpicking the call stack. `details` is free
/// text for any per-site context (call_id, ability name).
pub fn discard_try_send_classify<T>(
    result: Result<(), tokio::sync::mpsc::error::TrySendError<T>>,
    component: &'static str,
    details: &str,
) {
    match result {
        Ok(()) => {}
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            // Receiver gone — expected on disconnect, no-op.
        }
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            crate::op_event!(
                component = async_bridge,
                kind = try_send_dropped_full,
                level = "warn",
                emit_component = component,
                details = details,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_blocking_outside_tokio_uses_futures_executor() {
        // No tokio runtime; futures executor must drive the future.
        let v = run_blocking(async { 42_u32 }, NoRuntimeFallback::UseFuturesExecutor);
        assert_eq!(v, 42);
    }

    #[test]
    fn run_blocking_outside_tokio_can_build_current_thread() {
        // The build-tokio fallback must also work when called with
        // no ambient runtime — used by CLI bridge code.
        let v = run_blocking(async { 7_u32 }, NoRuntimeFallback::BuildCurrentThreadTokio);
        assert_eq!(v, 7);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_blocking_inside_current_thread_runtime_honors_build_tokio_fallback() {
        // This future requires a tokio reactor. A plain
        // futures::executor::block_on from the current-thread runtime
        // would panic or wedge; BuildCurrentThreadTokio must move it
        // onto a fresh runtime hosted by a helper thread.
        let v = run_blocking(
            async {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                11_u32
            },
            NoRuntimeFallback::BuildCurrentThreadTokio,
        );
        assert_eq!(v, 11);
    }

    #[test]
    fn try_run_blocking_outside_tokio_returns_none() {
        let v = try_run_blocking_in_tokio(async { 1_u32 });
        assert!(v.is_none());
    }

    #[test]
    fn try_run_blocking_outside_tokio_returns_result() {
        let v = try_run_blocking(
            async { 17_u32 },
            NoRuntimeFallback::BuildCurrentThreadTokio,
            "test bridge",
        )
        .expect("bridge result");
        assert_eq!(v, 17);
    }

    #[test]
    fn spawn_current_thread_tokio_runs_detached_future() {
        let (tx, rx) = std::sync::mpsc::channel();
        spawn_current_thread_tokio(
            "async-bridge-test",
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                tx.send(23_u32).expect("send result");
            },
            |error| panic!("runtime build failed: {error}"),
        )
        .expect("spawn current-thread tokio worker");

        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(2))
                .expect("detached worker result"),
            23
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_blocking_inside_multi_thread_runtime_works() {
        // Caller is on a multi-thread runtime; the helper should
        // defer to block_in_place + handle.block_on. The future
        // returns synchronously, so this just exercises the path.
        let v = tokio::task::spawn_blocking(|| {
            run_blocking(async { 99_u32 }, NoRuntimeFallback::UseFuturesExecutor)
        })
        .await
        .unwrap();
        assert_eq!(v, 99);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn try_run_blocking_inside_tokio_returns_some() {
        let v = tokio::task::spawn_blocking(|| try_run_blocking_in_tokio(async { 3_u32 }))
            .await
            .unwrap();
        assert_eq!(v, Some(3));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn try_run_blocking_inside_current_thread_runtime_supports_tokio_resources() {
        let v = try_run_blocking_in_tokio(async {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            5_u32
        });
        assert_eq!(v, Some(5));
    }
}
