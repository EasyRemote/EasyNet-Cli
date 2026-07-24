// Interpreter test suite (split from interpreter.rs, T4.4).

use super::retry::{
    compute_backoff, resolve_arguments, verify_output_done, ResolveError, VerifyDone,
    RETRY_BASE_MS, RETRY_MAX_MS,
};
use super::trace::{CappedTraceBuffer, CapturedResult, TRACE_CAP_HEAD, TRACE_CAP_TAIL};
use super::*;
use crate::eal::runtime::ir::{IrCall, IrFailurePolicy};
use std::collections::BTreeMap;

#[cfg(test)]
mod cases {
    use super::*;
    use crate::daemon::execution::child_invocation::{
        ChildInvocationReceiptAnchor, ChildInvocationRecord,
    };
    use crate::eal::{parser, runtime::planner};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    // ── Mock dispatcher for testing ──

    struct MockDispatcher {
        /// Per-call delay to simulate real work
        delay_ms: u64,
        /// Counter to track how many dispatch calls happened
        call_count: Arc<AtomicU32>,
        /// If set, fail the first N calls (for retry testing)
        fail_first_n: Arc<AtomicU32>,
        /// If set, fail calls whose function name is in this set
        fail_functions: Arc<std::collections::HashSet<String>>,
        /// Record of function names called (for ordering verification)
        calls: Arc<Mutex<Vec<(String, Instant)>>>,
        /// Record of run trace ids observed by dispatch calls.
        traces: Arc<Mutex<Vec<String>>>,
        /// Record of dispatch timeout budgets observed by calls.
        timeouts: Arc<Mutex<Vec<Option<u64>>>>,
    }

    impl MockDispatcher {
        fn new(delay_ms: u64) -> Self {
            Self {
                delay_ms,
                call_count: Arc::new(AtomicU32::new(0)),
                fail_first_n: Arc::new(AtomicU32::new(0)),
                fail_functions: Arc::new(std::collections::HashSet::new()),
                calls: Arc::new(Mutex::new(Vec::new())),
                traces: Arc::new(Mutex::new(Vec::new())),
                timeouts: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_fail_first_n(mut self, n: u32) -> Self {
            self.fail_first_n = Arc::new(AtomicU32::new(n));
            self
        }

        fn with_fail_functions(mut self, names: &[&str]) -> Self {
            self.fail_functions = Arc::new(names.iter().map(|s| (*s).to_string()).collect());
            self
        }
    }

    impl StepDispatcher for MockDispatcher {
        fn dispatch(
            &self,
            run: RunContext<'_>,
            _target: &IrTarget,
            ability: &AbilityName,
            _arguments: &Value,
            timeout_ms: Option<u64>,
            _dependency_receipts: &[ChildInvocationReceiptAnchor],
        ) -> Result<StepDispatchOutcome, EalError> {
            let ability_str = ability.as_str().to_string();
            let call_num = self.call_count.fetch_add(1, Ordering::SeqCst);
            self.calls
                .lock()
                .unwrap()
                .push((ability_str.clone(), Instant::now()));
            self.traces.lock().unwrap().push(run.trace_id.to_string());
            self.timeouts.lock().unwrap().push(timeout_ms);

            // Simulate work
            if self.delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(self.delay_ms));
            }

            // Fail by ability name (deterministic — safe for parallel tests)
            if self.fail_functions.contains(&ability_str) {
                return Err(EalError::Unavailable(format!(
                    "simulated failure for {ability_str}"
                )));
            }

            // Fail first N calls (order-dependent — use only in sequential phases)
            let fail_n = self.fail_first_n.load(Ordering::SeqCst);
            if call_num < fail_n {
                return Err(EalError::Unavailable(format!(
                    "simulated failure #{call_num}"
                )));
            }

            Ok(serde_json::json!({
                "ok": true,
                "call_num": call_num,
                "function": ability_str,
            })
            .into())
        }

        fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
            Ok(Box::new(MockDispatcher {
                delay_ms: self.delay_ms,
                call_count: Arc::clone(&self.call_count),
                fail_first_n: Arc::clone(&self.fail_first_n),
                fail_functions: Arc::clone(&self.fail_functions),
                calls: Arc::clone(&self.calls),
                traces: Arc::clone(&self.traces),
                timeouts: Arc::clone(&self.timeouts),
            }))
        }
    }

    // ── CappedTraceBuffer: bounded memory invariant ──

    /// Small helper: build a synthetic `StepTrace` with the given id,
    /// so buffer tests don't depend on executing a real mission.
    fn synth_trace(id: &str) -> StepTrace {
        StepTrace {
            step_id: id.to_string(),
            ability: crate::core::agent::id::AbilityName::parse("t").expect("valid ability name"),
            target: crate::eal::runtime::ir::IrTarget::Device {
                node_id: "n".to_string(),
            },
            phase_index: 0,
            started_at_unix_ms: 0,
            completed_at_unix_ms: 0,
            elapsed_ms: 0,
            outcome: StepOutcome::Completed,
            retry_count: 0,
            retry_history: vec![],
            result_size_bytes: None,
            result_sha256: None,
            invocation: None,
            error: None,
            input_refs: BTreeMap::new(),
            output_binding: None,
        }
    }

    #[test]
    fn capped_trace_buffer_under_head_cap_keeps_everything() {
        let mut buf = CappedTraceBuffer::new();
        for i in 0..10 {
            buf.push(synth_trace(&format!("s{i}")));
        }
        assert_eq!(buf.len(), 10);
        let (entries, dropped) = buf.into_parts();
        assert_eq!(dropped, 0);
        assert_eq!(entries.len(), 10);
        // Order preserved — this is the forensics contract.
        for (i, e) in entries.iter().enumerate() {
            assert_eq!(e.step_id, format!("s{i}"));
        }
    }

    #[test]
    fn explicit_trace_id_reaches_dispatch_and_report() {
        let ir = planner::compile(
            &parser::parse(r#"mission "trace-contract" { let r = alice.chat(prompt: "hi") }"#)
                .unwrap(),
        )
        .unwrap();
        let dispatcher = MockDispatcher::new(0);
        let observed = Arc::clone(&dispatcher.traces);
        let report =
            execute_with_dispatcher_for_trace(&dispatcher, "test", &ir, "run-trace-42".into())
                .unwrap();

        assert_eq!(report.trace.mission_id, "run-trace-42");
        assert_eq!(
            *observed.lock().unwrap(),
            vec!["run-trace-42".to_string()],
            "child dispatch must carry the caller-owned mission trace id"
        );
    }

    #[test]
    fn run_deadline_bounds_step_dispatch_timeout() {
        let ir = planner::compile(
            &parser::parse(r#"mission "deadline" { let r = call "slow.op" on "n1" timeout 120 }"#)
                .unwrap(),
        )
        .unwrap();
        let dispatcher = MockDispatcher::new(0);

        let report = execute_with_dispatcher_for_trace_with_timeout(
            &dispatcher,
            "test",
            &ir,
            "deadline-run".into(),
            Some(Duration::from_millis(500)),
        )
        .unwrap();

        assert_eq!(report.steps_failed, 0);
        let observed = dispatcher.timeouts.lock().unwrap().clone();
        assert_eq!(observed.len(), 1);
        let timeout = observed[0].expect("run deadline must produce dispatch timeout");
        assert!(
            timeout <= 500 && timeout > 0,
            "dispatch timeout must be clipped to remaining run deadline, got {timeout}"
        );
    }

    #[test]
    fn emit_records_resolved_binding_and_literal_without_extra_dispatch() {
        let ir = planner::compile(
            &parser::parse(
                r#"mission "emit-contract" {
                    let rows = alice.produce(prompt: "hi")
                    emit "terminal_rows" kind answer value rows.output
                    emit "operator_chain" kind context value "produce"
                }"#,
            )
            .unwrap(),
        )
        .unwrap();
        let dispatcher = MockDispatcher::new(0);
        let calls = Arc::clone(&dispatcher.call_count);
        let report =
            execute_with_dispatcher_for_trace(&dispatcher, "test", &ir, "run-emit-1".into())
                .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1, "emit must not dispatch");
        assert_eq!(report.trace.emissions.len(), 2);
        assert_eq!(report.trace.emissions[0].seq, 1);
        assert_eq!(report.trace.emissions[0].name, "terminal_rows");
        assert_eq!(report.trace.emissions[0].kind, "answer");
        assert_eq!(
            report.trace.emissions[0].source_binding.as_deref(),
            Some("rows")
        );
        assert_eq!(report.trace.emissions[0].value["function"], "produce");
        assert!(report.trace.emissions[0].error.is_none());
        assert_eq!(report.trace.emissions[1].value, "produce");
    }

    #[test]
    fn emit_records_missing_binding_when_producer_fails() {
        let ir = planner::compile(
            &parser::parse(
                r#"mission "emit-missing" {
                    let rows = alice.fail(prompt: "hi")
                    emit "terminal_rows" kind answer value rows.output
                }"#,
            )
            .unwrap(),
        )
        .unwrap();
        let dispatcher = MockDispatcher::new(0).with_fail_functions(&["fail"]);
        let report =
            execute_with_dispatcher_for_trace(&dispatcher, "test", &ir, "run-emit-2".into())
                .unwrap();

        assert_eq!(report.trace.outcome, MissionOutcome::Partial);
        assert_eq!(report.trace.emissions.len(), 1);
        assert_eq!(report.trace.emissions[0].value, serde_json::Value::Null);
        assert_eq!(
            report.trace.emissions[0].source_binding.as_deref(),
            Some("rows")
        );
        assert!(
            report.trace.emissions[0]
                .error
                .as_deref()
                .is_some_and(|msg| msg.contains("was not captured")),
            "missing binding must be explicit in the emission record"
        );
    }

    #[test]
    fn capped_trace_buffer_head_boundary_saturates_exactly() {
        // Pushing exactly TRACE_CAP_HEAD entries must fill the head
        // and leave the tail empty — no entries dropped, no tail use.
        let mut buf = CappedTraceBuffer::new();
        for i in 0..TRACE_CAP_HEAD {
            buf.push(synth_trace(&format!("s{i}")));
        }
        let (entries, dropped) = buf.into_parts();
        assert_eq!(dropped, 0);
        assert_eq!(entries.len(), TRACE_CAP_HEAD);
    }

    #[test]
    fn capped_trace_buffer_between_head_and_cap_uses_tail() {
        // Head + part of tail, but within TRACE_CAP_TOTAL — nothing
        // should be dropped.
        let n = TRACE_CAP_HEAD + 50;
        let mut buf = CappedTraceBuffer::new();
        for i in 0..n {
            buf.push(synth_trace(&format!("s{i}")));
        }
        let (entries, dropped) = buf.into_parts();
        assert_eq!(dropped, 0);
        assert_eq!(entries.len(), n);
    }

    #[test]
    fn capped_trace_buffer_over_cap_drops_middle_with_count() {
        // Push twice TRACE_CAP_TOTAL. Expect: head preserved, tail
        // holds the most recent TRACE_CAP_TAIL entries, middle slab
        // counted as dropped.
        let n = TRACE_CAP_HEAD + TRACE_CAP_TAIL + 250;
        let expected_dropped = n - (TRACE_CAP_HEAD + TRACE_CAP_TAIL);
        let mut buf = CappedTraceBuffer::new();
        for i in 0..n {
            buf.push(synth_trace(&format!("s{i}")));
        }
        let (entries, dropped) = buf.into_parts();
        assert_eq!(dropped, expected_dropped);
        assert_eq!(entries.len(), TRACE_CAP_HEAD + TRACE_CAP_TAIL);

        // Head first TRACE_CAP_HEAD entries are s0..s{HEAD-1}.
        for (i, e) in entries.iter().take(TRACE_CAP_HEAD).enumerate() {
            assert_eq!(e.step_id, format!("s{i}"));
        }
        // Tail last TRACE_CAP_TAIL entries are s{n-TAIL}..s{n-1}.
        let tail_slice = &entries[TRACE_CAP_HEAD..];
        for (offset, e) in tail_slice.iter().enumerate() {
            let expected_idx = n - TRACE_CAP_TAIL + offset;
            assert_eq!(e.step_id, format!("s{expected_idx}"));
        }
    }

    // ── Test 1: Parallel dispatch actually runs concurrently ──

    #[test]
    fn parallel_workers_inherit_the_mission_context() {
        // F-028 / T5.4: rayon workers must see the orchestrating
        // thread's DispatchContext via the explicit handoff — NOT via
        // a process-global env var (which concurrent missions stomp).
        struct ContextProbe {
            seen: Arc<Mutex<Vec<Option<String>>>>,
        }
        impl StepDispatcher for ContextProbe {
            fn dispatch(
                &self,
                _run: RunContext<'_>,
                _target: &IrTarget,
                _ability: &AbilityName,
                _arguments: &Value,
                _timeout_ms: Option<u64>,
                _dependency_receipts: &[ChildInvocationReceiptAnchor],
            ) -> Result<StepDispatchOutcome, EalError> {
                self.seen.lock().unwrap().push(
                    crate::daemon::execution::mission::context::current().map(|c| c.mission_id),
                );
                Ok(StepDispatchOutcome::from(serde_json::json!({})))
            }
            fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
                Ok(Box::new(ContextProbe {
                    seen: Arc::clone(&self.seen),
                }))
            }
        }

        let src = r#"
            mission "ctx-handoff" {
                let a = call "slow.op" on "n1"
                let b = call "slow.op" on "n2"
                let c = call "slow.op" on "n3"
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let dispatcher = ContextProbe {
            seen: Arc::clone(&seen),
        };
        let _ctx = crate::daemon::execution::mission::context::enter(
            crate::daemon::execution::mission::context::DispatchContext::for_mission(
                "ctx-handoff-run",
                std::env::temp_dir(),
            ),
        );
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();
        assert_eq!(report.steps_completed, 3);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        for ctx in seen.iter() {
            assert_eq!(
                ctx.as_deref(),
                Some("ctx-handoff-run"),
                "every parallel worker must inherit the mission context"
            );
        }
    }

    #[test]
    fn parallel_dispatch_is_concurrent() {
        // 3 independent steps, each takes 100ms.
        // If sequential: ≥300ms. If parallel: ~100ms.
        let src = r#"
            mission "parallel-test" {
                let a = call "slow.op" on "n1"
                let b = call "slow.op" on "n2"
                let c = call "slow.op" on "n3"
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();

        // All 3 steps should be in phase 0 (independent)
        assert_eq!(ir.phases.len(), 1);

        let dispatcher = MockDispatcher::new(100);
        let t0 = Instant::now();
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();
        let elapsed = t0.elapsed();

        assert_eq!(report.steps_completed, 3);
        assert_eq!(report.steps_failed, 0);
        assert_eq!(dispatcher.call_count.load(Ordering::SeqCst), 3);

        // Must finish in under 250ms (3×100ms serial would be ≥300ms)
        assert!(
            elapsed < Duration::from_millis(250),
            "parallel dispatch took {elapsed:?} — expected <250ms for 3×100ms steps"
        );
    }

    // ── Test 2: Sequential phases respect data dependencies ──

    #[test]
    fn sequential_phases_respect_order() {
        let src = r#"
            mission "chain" {
                let a = call "step1" on "n1"
                let b = call "step2" on "n1" with { input = a.output }
                let c = call "step3" on "n1" with { input = b.output }
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        assert_eq!(ir.phases.len(), 3);

        let dispatcher = MockDispatcher::new(10);
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

        assert_eq!(report.steps_completed, 3);

        // Verify call ordering
        let calls = dispatcher.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].0, "step1");
        assert_eq!(calls[1].0, "step2");
        assert_eq!(calls[2].0, "step3");
        // Each call started after the previous one finished
        assert!(calls[1].1 > calls[0].1);
        assert!(calls[2].1 > calls[1].1);
    }

    // ── Test 3: Execution trace captures correct fields ──

    #[test]
    fn trace_captures_fields() {
        let src = r#"
            mission "traced" {
                let x = call "compute" on "gpu"
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();

        let dispatcher = MockDispatcher::new(0);
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();
        let trace = &report.trace;

        assert_eq!(trace.mission_name, "traced");
        assert!(!trace.mission_id.is_empty());
        assert_eq!(trace.phase_count, 1);
        assert_eq!(trace.steps_completed, 1);
        assert_eq!(trace.steps_failed, 0);
        assert_eq!(trace.outcome, MissionOutcome::Completed);
        assert!(trace.started_at_unix_ms > 0);
        assert!(trace.completed_at_unix_ms >= trace.started_at_unix_ms);

        let st = &trace.step_traces[0];
        assert_eq!(st.step_id, "x");
        assert_eq!(st.ability.as_str(), "compute");
        assert_eq!(
            st.target,
            IrTarget::Device {
                node_id: "gpu".to_string()
            }
        );
        assert_eq!(st.phase_index, 0);
        assert_eq!(st.outcome, StepOutcome::Completed);
        assert!(st.result_size_bytes.unwrap() > 0);
        assert!(st.result_sha256.is_some());
        assert!(st.error.is_none());
    }

    // ── Test 4: Trace is serializable to JSON ──

    #[test]
    fn trace_is_serializable() {
        let src = r#"mission "s" { let a = call "x" on "n" }"#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        let dispatcher = MockDispatcher::new(0);
        let report = execute_with_dispatcher(&dispatcher, "t", &ir).unwrap();
        let json = serde_json::to_string_pretty(&report.trace).unwrap();
        assert!(json.contains("\"mission_name\": \"s\""));
        assert!(json.contains("\"result_sha256\""));
        // Roundtrip
        let _: ExecutionTrace = serde_json::from_str(&json).unwrap();
    }

    // ── Test 5: Retry with exponential backoff ──

    #[test]
    fn retry_fires_correct_attempts() {
        let src = r#"
            mission "retry-test" {
                let x = call "flaky" on "n" retries 3 on_failure retry
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();

        // Fail first 2 calls, succeed on 3rd
        let dispatcher = MockDispatcher::new(0).with_fail_first_n(2);
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

        assert_eq!(report.steps_completed, 1);
        assert_eq!(report.steps_failed, 0);
        // Total calls: 2 failures + 1 success = 3
        assert_eq!(dispatcher.call_count.load(Ordering::SeqCst), 3);

        let st = &report.trace.step_traces[0];
        assert_eq!(st.outcome, StepOutcome::Completed);
        assert_eq!(st.retry_count, 2); // 2 retries before success
        assert_eq!(st.retry_history.len(), 2);
        assert!(st.retry_history[0].error.contains("simulated failure"));
    }

    // ── Test 6: Retry exhaustion results in failure ──

    #[test]
    fn retry_exhaustion_fails() {
        let src = r#"
            mission "exhaust" {
                let x = call "always-fail" on "n" retries 2 on_failure retry
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();

        // Fail all calls
        let dispatcher = MockDispatcher::new(0).with_fail_first_n(100);
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

        assert_eq!(report.steps_completed, 0);
        assert_eq!(report.steps_failed, 1);
        assert_eq!(report.trace.outcome, MissionOutcome::Partial);
        // max_retries=2 means 3 total attempts (1 + 2 retries)
        assert_eq!(dispatcher.call_count.load(Ordering::SeqCst), 3);

        let st = &report.trace.step_traces[0];
        assert_eq!(st.outcome, StepOutcome::Failed);
        assert!(st.error.is_some());
        assert_eq!(st.retry_count, 2);
        assert_eq!(st.retry_history.len(), 3); // all attempts failed
    }

    // ── Test 7: Abort policy stops execution ──

    #[test]
    fn abort_stops_subsequent_phases() {
        let src = r#"
            mission "abort-test" {
                let a = call "will-fail" on "n" on_failure abort
                let b = call "should-not-run" on "n" with { input = a.output }
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        assert_eq!(ir.phases.len(), 2);

        let dispatcher = MockDispatcher::new(0).with_fail_first_n(1);
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

        assert_eq!(report.steps_failed, 1);
        assert_eq!(report.trace.outcome, MissionOutcome::Aborted);
        // Only 1 call made (step b never dispatched)
        assert_eq!(dispatcher.call_count.load(Ordering::SeqCst), 1);
        assert_eq!(report.trace.step_traces.len(), 1);
    }

    // ── Test 8: Optional step failure doesn't abort ──

    #[test]
    fn optional_step_skipped() {
        let src = r#"
            mission "opt" {
                call "maybe" on "n" optional
                let b = call "must-run" on "n"
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();

        // Both steps land in the same phase (no data dependency).
        // Scheduling priority: required ("must-run") executes first → call #0.
        // Optional ("maybe") executes second → call #1.
        // fail_first_n(1) fails call #0 ("must-run"), which is not optional → Failed.
        //
        // But the test expects the *optional* step to fail and be skipped.
        // We need fail_first_n(2) so both fail, then:
        //   must-run (required, call #0) → Failed → abort? No: default on_failure is Continue.
        //   maybe (optional, call #1) → Skipped.
        //
        // Actually: the correct test is to fail only the optional step.
        // With priority scheduling, optional runs second (call #1).
        // fail_first_n only fails call #0, which hits must-run.
        // To fail only the optional step, use with_fail_functions.
        let dispatcher = MockDispatcher::new(0).with_fail_functions(&["maybe"]);
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

        assert_eq!(report.steps_completed, 1, "must-run should succeed");
        assert_eq!(
            report.trace.steps_skipped, 1,
            "optional failure should be skipped"
        );
        assert_eq!(report.trace.outcome, MissionOutcome::Completed);
    }

    /// Regression: when an optional step is skipped and a downstream
    /// step consumes its output, the downstream must propagate as
    /// `Skipped` (not `Failed`). Previously the missing binding hit
    /// the `unresolved ref` branch and the consumer was classified
    /// `Failed`, which miscategorised "my producer didn't run" as
    /// "I ran and failed" in the trace — confusing operators reading
    /// the audit log.
    #[test]
    fn downstream_is_auto_skipped_when_its_producer_is_skipped() {
        let src = r#"
            mission "cascade-skip" {
                let p = call "producer" on "n" optional
                let c = call "consumer" on "n" with { input = p.output }
            }
        "#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();

        // `producer` fails (and is optional → Skipped), so `consumer`
        // has no `p` binding to read. With the cascade-skip fix, the
        // consumer sees `ResolveError::UpstreamSkipped` and is
        // classified as Skipped too.
        let dispatcher = MockDispatcher::new(0).with_fail_functions(&["producer"]);
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

        assert_eq!(
            report.trace.steps_skipped, 2,
            "both the optional producer and the dependent consumer must skip; got trace: {:?}",
            report.trace.step_traces
        );
        assert_eq!(report.steps_completed, 0);
        assert_eq!(report.steps_failed, 0);

        // Consumer trace carries the provenance in its error message.
        let consumer = report
            .trace
            .step_traces
            .iter()
            .find(|t| t.ability.as_str() == "consumer")
            .expect("consumer trace must be present");
        assert_eq!(consumer.outcome, StepOutcome::Skipped);
        let err = consumer
            .error
            .as_deref()
            .expect("cascaded skip must carry a provenance message");
        assert!(
            err.contains("`p`"),
            "cascaded-skip message must name the missing upstream binding; got: {err}"
        );
    }

    // ── Test 9: Diamond graph phases + data flow ──

    #[test]
    fn diamond_parallel_phases() {
        let src = r#"
            mission "diamond" {
                let a = call "root" on "n1"
                let b = call "left" on "n2" with { input = a.output }
                let c = call "right" on "n3" with { input = a.output }
                let d = call "merge" on "n4" with { l = b.output, r = c.output }
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        assert_eq!(ir.phases.len(), 3);

        // Phase 1 (b,c) should run in parallel. Use enough per-step
        // delay that the serial-vs-parallel gap is larger than normal
        // test-runtime scheduling overhead.
        let dispatcher = MockDispatcher::new(100);
        let t0 = Instant::now();
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();
        let elapsed = t0.elapsed();

        assert_eq!(report.steps_completed, 4);
        // Phase 0: 100ms, Phase 1: 100ms (parallel b+c), Phase 2:
        // 100ms = ~300ms. If b,c were serial: ~400ms plus runtime
        // scheduling overhead.
        assert!(
            elapsed < Duration::from_millis(450),
            "diamond took {elapsed:?} — parallel phase should save time"
        );
    }

    // ── Test 10: Backoff calculation is deterministic and exponential ──
    //
    // These tests pin three properties the retry scheduler relies on:
    //
    //   1. **Exponential growth** (attempt N doubles attempt N-1's base).
    //   2. **Upper bound** (capped base + bounded jitter; never unbounded).
    //   3. **Determinism** (same `(attempt, step_id)` → same delay across
    //      runs, threads, and processes). Determinism lets replay-based
    //      trace comparison (two runs of the same mission) line up
    //      exactly, which is the whole point of the deterministic jitter.
    //
    // Jitter bound: `jitter_seed % (RETRY_BASE_MS / 2 + 1)` → jitter is
    // in `0..=500` ms. Asserting the *strict* upper bound (not just
    // "capped + BASE") is what turns "works today" into "contract".

    #[test]
    fn backoff_is_exponential_and_deterministic() {
        let b1 = compute_backoff(1, "step-a");
        let b2 = compute_backoff(2, "step-a");
        let b3 = compute_backoff(3, "step-a");

        // Base: 1000 ms. Attempt N adds `BASE * 2^(N-1)` (capped at MAX)
        // plus a jitter of `0..=BASE/2`. The lower bound is the pure
        // exponential; the upper bound is cap + jitter_max.
        let jitter_max = RETRY_BASE_MS / 2;
        assert!(b1 >= RETRY_BASE_MS && b1 <= RETRY_BASE_MS + jitter_max);
        assert!(b2 >= RETRY_BASE_MS * 2 && b2 <= RETRY_BASE_MS * 2 + jitter_max);
        assert!(b3 >= RETRY_BASE_MS * 4 && b3 <= RETRY_BASE_MS * 4 + jitter_max);

        // Determinism: same inputs → same output, same process OR fresh.
        assert_eq!(b1, compute_backoff(1, "step-a"));
        assert_eq!(b2, compute_backoff(2, "step-a"));
        assert_eq!(b3, compute_backoff(3, "step-a"));

        // Different step_id → independent jitter (but still in range).
        let b1_other = compute_backoff(1, "step-b");
        assert!(b1_other >= RETRY_BASE_MS && b1_other <= RETRY_BASE_MS + jitter_max);
    }

    /// Capping behaviour: beyond the saturation attempt, `base` is
    /// clamped at `RETRY_MAX_MS` and the only variation comes from
    /// jitter. A future refactor that removed the `min(MAX)` would
    /// send delays into the stratosphere and fail this test.
    #[test]
    fn backoff_caps_base_at_retry_max_ms() {
        let jitter_max = RETRY_BASE_MS / 2;
        // attempt=10 → raw base = 1000 * 2^9 = 512_000, well past MAX=30_000
        let capped = compute_backoff(10, "saturating-step");
        assert!(
            capped >= RETRY_MAX_MS && capped <= RETRY_MAX_MS + jitter_max,
            "attempt=10 must saturate at RETRY_MAX_MS (~{RETRY_MAX_MS}); got {capped}"
        );
    }

    /// Cross-step independence: two different step ids at the same
    /// attempt number must (with overwhelming probability) yield
    /// different jitter values. Asserting *any difference* across a
    /// small corpus is a cheap way to catch a regression that silently
    /// collapsed jitter to a constant (e.g. forgot to mix in step_id).
    #[test]
    fn backoff_jitter_varies_across_step_ids() {
        let values: std::collections::HashSet<u64> = (0..8)
            .map(|i| compute_backoff(1, &format!("s{i}")))
            .collect();
        assert!(
            values.len() > 1,
            "jitter collapsed to a constant across step ids — SHA256 seed broken?"
        );
    }

    // ── Test 11: Graceful fallback to sequential when clone_for_thread fails ──

    #[test]
    fn fallback_to_sequential_when_not_cloneable() {
        // Non-cloneable dispatcher simulates a production dispatcher that
        // cannot safely cross rayon worker threads.
        struct SeqOnlyDispatcher(Arc<AtomicU32>);
        impl StepDispatcher for SeqOnlyDispatcher {
            fn dispatch(
                &self,
                _: RunContext<'_>,
                _target: &IrTarget,
                ability: &AbilityName,
                _: &Value,
                _: Option<u64>,
                _: &[ChildInvocationReceiptAnchor],
            ) -> Result<StepDispatchOutcome, EalError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(serde_json::json!({"ok": true, "function": ability.as_str()}).into())
            }
            fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
                Err(EalError::Internal("not cloneable".into()))
            }
        }

        // 3 independent steps — normally would run in parallel
        let src = r#"mission "f" { let a = call "x" on "n1" let b = call "y" on "n2" let c = call "z" on "n3" }"#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        assert_eq!(ir.phases.len(), 1);

        let count = Arc::new(AtomicU32::new(0));
        let dispatcher = SeqOnlyDispatcher(Arc::clone(&count));
        let report = execute_with_dispatcher(&dispatcher, "t", &ir).unwrap();

        // All 3 steps succeed despite no parallel support
        assert_eq!(report.steps_completed, 3);
        assert_eq!(report.steps_failed, 0);
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    // ── Test 12: Cross-phase data flow propagates results correctly ──

    #[test]
    fn cross_phase_data_flow() {
        let src = r#"
            mission "flow" {
                let a = call "produce" on "n1"
                let b = call "consume" on "n2" with { input = a.output }
            }
        "#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        assert_eq!(ir.phases.len(), 2);

        let dispatcher = MockDispatcher::new(0);
        let report = execute_with_dispatcher(&dispatcher, "t", &ir).unwrap();

        assert_eq!(report.steps_completed, 2);
        // Verify trace shows data flow connections
        let traces = &report.trace.step_traces;
        assert_eq!(traces[0].output_binding, Some("a".into()));
        assert!(traces[0].result_sha256.is_some());
        assert_eq!(traces[1].input_refs.get("input"), Some(&"a".into()));
        // Both steps have result hashes (proving they executed and returned data)
        assert!(traces[1].result_sha256.is_some());
    }

    // ── Surface form → IR target asymmetry (anti-regression) ───────────────
    //
    // The two EAL surface forms intentionally lower to DIFFERENT IR
    // target variants:
    //
    //   member-call form  `claude.chat(prompt: "hi")`
    //     → IrTarget::Agent(AgentId { tenant: "default", name: "claude" })
    //
    //   traditional form  `call "chat" on "claude" with { prompt = "hi" }`
    //     → IrTarget::Device { node_id: "claude" }
    //
    // The asymmetry is the design (ontology §5: device is hosting
    // substrate, §6.4: agent is logical actor; surface forms encode
    // the distinction). The runtime dispatcher matches `IrTarget`
    // and never re-classifies. See AGENT_IDENTITY.md invariant 2.

    /// Dispatcher that records every `(target, ability, args)` tuple
    /// it receives. Used to verify the resolved dispatch shapes.
    struct ShapeRecordingDispatcher {
        seen: Arc<Mutex<Vec<(IrTarget, AbilityName, Value)>>>,
    }

    impl ShapeRecordingDispatcher {
        fn new() -> Self {
            Self {
                seen: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl StepDispatcher for ShapeRecordingDispatcher {
        fn dispatch(
            &self,
            _run: RunContext<'_>,
            target: &IrTarget,
            ability: &AbilityName,
            arguments: &Value,
            _timeout_ms: Option<u64>,
            _dependency_receipts: &[ChildInvocationReceiptAnchor],
        ) -> Result<StepDispatchOutcome, EalError> {
            self.seen
                .lock()
                .unwrap()
                .push((target.clone(), ability.clone(), arguments.clone()));
            Ok(serde_json::json!({"ok": true}).into())
        }

        fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
            Ok(Box::new(ShapeRecordingDispatcher {
                seen: Arc::clone(&self.seen),
            }))
        }
    }

    /// Regression: when a dispatcher returns a categorised `EalError`,
    /// the `error_code:` prefix must survive the boundary into the
    /// trace and retry log. Operators reading a trace file should be
    /// able to grep for `validation_error:` / `not_found:` /
    /// `unavailable:` / `internal_error:` without needing the typed
    /// error available — that is the whole point of using `Display`
    /// (rather than just `.message()`) at the boundary.
    #[test]
    fn dispatcher_error_code_is_preserved_in_trace_message() {
        struct CategorisedDispatcher;
        impl StepDispatcher for CategorisedDispatcher {
            fn dispatch(
                &self,
                _run: RunContext<'_>,
                _target: &IrTarget,
                _ability: &AbilityName,
                _arguments: &Value,
                _timeout_ms: Option<u64>,
                _dependency_receipts: &[ChildInvocationReceiptAnchor],
            ) -> Result<StepDispatchOutcome, EalError> {
                Err(EalError::NotFound("device 'node-x' not registered".into()))
            }
            fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
                Ok(Box::new(CategorisedDispatcher))
            }
        }

        let src = r#"mission "t" { let r = call "ping" on "node-x" }"#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        let report = execute_with_dispatcher(&CategorisedDispatcher, "tenant", &ir).unwrap();

        let trace = &report.trace.step_traces[0];
        let err_msg = trace.error.as_deref().expect("step must have an error");
        assert!(
            err_msg.starts_with("not_found:"),
            "trace error must start with the EalError code prefix; got: {err_msg}"
        );
        assert!(
            err_msg.contains("device 'node-x' not registered"),
            "trace error must include the human message; got: {err_msg}"
        );
    }

    /// Golden test: pin the on-disk JSON shape of `ExecutionTrace` v1.
    ///
    /// This test is the contract between this module and any external
    /// consumer that reads trace files (CI scrapers, external auditors,
    /// the future trace-replay UI). Renaming a field, removing one, or
    /// changing a numeric type fails this test — at which point the
    /// codebase is telling you to bump `EXECUTION_TRACE_SCHEMA_VERSION`
    /// and write an explicit reader-side migration.
    ///
    /// We assert two properties:
    ///   1. A freshly constructed trace serializes with
    ///      `schema_version = EXECUTION_TRACE_SCHEMA_VERSION` and the
    ///      full set of expected top-level keys.
    ///   2. A JSON payload without `schema_version` fails closed instead
    ///      of being inferred as the current schema.
    #[test]
    fn trace_schema_v1_is_stable() {
        // Use a hand-built trace rather than running a real mission so
        // the expected JSON is fully deterministic — no timestamps to
        // freeze, no SHA digests to mock.
        let trace = ExecutionTrace {
            schema_version: EXECUTION_TRACE_SCHEMA_VERSION,
            mission_id: "m-test".to_string(),
            mission_name: "test-mission".to_string(),
            started_at_unix_ms: 1_000,
            completed_at_unix_ms: 2_000,
            total_elapsed_ms: 1_000,
            phase_count: 1,
            steps_completed: 0,
            steps_failed: 0,
            steps_skipped: 0,
            outcome: MissionOutcome::Completed,
            step_traces: vec![],
            ability_graph: vec![],
            emissions: vec![],
            traces_truncated: 0,
        };

        let json: serde_json::Value =
            serde_json::to_value(&trace).expect("trace must serialize cleanly");

        // Property 1: version is stamped and the key set is fixed.
        assert_eq!(json["schema_version"], serde_json::json!(1));
        let expected_keys: std::collections::BTreeSet<&str> = [
            "schema_version",
            "mission_id",
            "mission_name",
            "started_at_unix_ms",
            "completed_at_unix_ms",
            "total_elapsed_ms",
            "phase_count",
            "steps_completed",
            "steps_failed",
            "steps_skipped",
            "outcome",
            "step_traces",
            // `traces_truncated` serializes as `0` for missions under
            // the cap. Its presence here pins that the on-the-wire
            // shape includes it for every fresh trace.
            "traces_truncated",
        ]
        .into_iter()
        .collect();
        let actual_keys: std::collections::BTreeSet<&str> = json
            .as_object()
            .expect("trace serializes to an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            actual_keys, expected_keys,
            "ExecutionTrace key set drift detected. \
             If this is intentional, bump EXECUTION_TRACE_SCHEMA_VERSION \
             and update this test."
        );

        // Property 2: payloads without `schema_version` fail closed.
        let missing_version_json = serde_json::json!({
            "mission_id": "missing-version",
            "mission_name": "missing-version",
            "started_at_unix_ms": 0,
            "completed_at_unix_ms": 0,
            "total_elapsed_ms": 0,
            "phase_count": 0,
            "steps_completed": 0,
            "steps_failed": 0,
            "steps_skipped": 0,
            "outcome": "completed",
            "step_traces": [],
        });
        let err = serde_json::from_value::<ExecutionTrace>(missing_version_json)
            .expect_err("trace JSON without schema_version must fail closed");
        assert!(
            err.to_string().contains("schema_version"),
            "strict schema error should name schema_version: {err}"
        );
    }

    /// Regression: a malformed upstream payload must fail the
    /// consuming step at `resolve_arguments`, not silently inject
    /// `null` and let the downstream step run with corrupt input. The
    /// previous `unwrap_or(Value::Null)` made this class of bug a
    /// debugging black hole — see commit history for the original
    /// motivation.
    #[test]
    fn resolve_arguments_fails_loud_on_malformed_upstream_payload() {
        use crate::core::agent::id::{AbilityName, AgentId};
        use std::collections::BTreeMap;

        let mut input_refs = BTreeMap::new();
        input_refs.insert("input".to_string(), "upstream".to_string());

        let step = IrCall {
            step_id: "consumer".to_string(),
            step_name: "consumer".to_string(),
            ability: AbilityName::parse("review").unwrap(),
            target: IrTarget::Agent(AgentId::parse("claude").unwrap()),
            static_arguments: serde_json::json!({}),
            input_refs,
            output_binding: None,
            timeout_seconds: 0,
            max_retries: 0,
            on_failure: IrFailurePolicy::Continue,
            optional: false,
            content_type: "application/json".to_string(),
        };

        // "{not json" is exactly the sort of partial / corrupted output
        // that motivated this guard — a streaming ability that died
        // mid-flush can leave bytes like this in the captured slot.
        let mut results: HashMap<String, CapturedResult> = HashMap::new();
        results.insert(
            "upstream".to_string(),
            CapturedResult {
                value: b"{not json".to_vec(),
                invocation: ChildInvocationRecord::for_test("test.malformed", 0x61),
            },
        );

        let skipped: std::collections::HashSet<String> = std::collections::HashSet::new();
        let err = resolve_arguments(&step, &results, &skipped)
            .expect_err("malformed upstream payload must surface as step error");
        let msg = err.to_string();
        // The malformed-payload path is `ResolveError::Other` (not
        // UpstreamSkipped) — the binding *was* produced, we just
        // couldn't parse the bytes.
        assert!(
            matches!(err, ResolveError::Other(_)),
            "malformed payload is a generic resolve error, not UpstreamSkipped; got: {err:?}"
        );
        assert!(
            msg.contains("input ref `input`"),
            "error must name the consuming arg name; got: {msg}"
        );
        assert!(
            msg.contains("binding `upstream`"),
            "error must name the upstream binding; got: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("not valid json"),
            "error must explain the failure category; got: {msg}"
        );
    }

    /// Regression: when an upstream step is skipped, a consumer's
    /// `resolve_arguments` must return the typed `UpstreamSkipped`
    /// variant (not the generic `Other`), so the caller can propagate
    /// `Skipped` instead of miscategorising as `Failed`. This is the
    /// contract that prevents the "optional producer → required
    /// consumer" trace from looking like a cascade of failures.
    #[test]
    fn resolve_arguments_returns_upstream_skipped_for_skipped_binding() {
        use crate::core::agent::id::{AbilityName, AgentId};
        use std::collections::BTreeMap;

        let mut input_refs = BTreeMap::new();
        input_refs.insert("input".to_string(), "producer".to_string());

        let step = IrCall {
            step_id: "consumer".to_string(),
            step_name: "consumer".to_string(),
            ability: AbilityName::parse("review").unwrap(),
            target: IrTarget::Agent(AgentId::parse("claude").unwrap()),
            static_arguments: serde_json::json!({}),
            input_refs,
            output_binding: None,
            timeout_seconds: 0,
            max_retries: 0,
            on_failure: IrFailurePolicy::Continue,
            optional: false,
            content_type: "application/json".to_string(),
        };

        let results: HashMap<String, CapturedResult> = HashMap::new();
        let mut skipped = std::collections::HashSet::new();
        skipped.insert("producer".to_string());

        let err = resolve_arguments(&step, &results, &skipped)
            .expect_err("skipped upstream must surface as typed skip");
        match err {
            ResolveError::UpstreamSkipped { binding, arg } => {
                assert_eq!(binding, "producer");
                assert_eq!(arg, "input");
            }
            other => panic!("expected UpstreamSkipped, got: {other:?}"),
        }
    }

    /// Counterpart: well-formed payloads still flow through cleanly.
    /// Pinned alongside the failure case so a future refactor that
    /// over-tightens the parser (e.g. requires top-level objects) is
    /// caught immediately.
    #[test]
    fn resolve_arguments_threads_well_formed_payload() {
        use crate::core::agent::id::{AbilityName, AgentId};
        use std::collections::BTreeMap;

        let mut input_refs = BTreeMap::new();
        input_refs.insert("input".to_string(), "upstream".to_string());

        let step = IrCall {
            step_id: "consumer".to_string(),
            step_name: "consumer".to_string(),
            ability: AbilityName::parse("review").unwrap(),
            target: IrTarget::Agent(AgentId::parse("claude").unwrap()),
            static_arguments: serde_json::json!({"k": "static"}),
            input_refs,
            output_binding: None,
            timeout_seconds: 0,
            max_retries: 0,
            on_failure: IrFailurePolicy::Continue,
            optional: false,
            content_type: "application/json".to_string(),
        };

        let mut results: HashMap<String, CapturedResult> = HashMap::new();
        results.insert(
            "upstream".to_string(),
            CapturedResult {
                value: b"{\"answer\": 42}".to_vec(),
                invocation: ChildInvocationRecord::for_test("test.producer", 0x62),
            },
        );

        let skipped: std::collections::HashSet<String> = std::collections::HashSet::new();
        let resolved =
            resolve_arguments(&step, &results, &skipped).expect("well-formed payload must parse");
        let obj = resolved.as_object().expect("resolved args are an object");
        assert_eq!(obj.get("k"), Some(&serde_json::json!("static")));
        assert_eq!(obj.get("input"), Some(&serde_json::json!({"answer": 42})));
    }

    #[test]
    fn member_call_lowers_to_agent_target() {
        use crate::core::agent::id::AgentId;

        let src = r#"
            mission "member-call" {
                let r = claude.chat(prompt: "hi")
            }
        "#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        assert_eq!(ir.steps.len(), 1);
        let call = ir.steps[0].as_call().expect("flat call step");
        assert_eq!(call.ability.as_str(), "chat");
        assert_eq!(
            call.target,
            IrTarget::Agent(AgentId::parse("claude").unwrap()),
            "member-call must lower to IrTarget::Agent"
        );
    }

    #[test]
    fn traditional_call_lowers_to_device_target() {
        let src = r#"
            mission "traditional" {
                let r = call "chat" on "node-1" with { prompt = "hi" }
            }
        "#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        assert_eq!(ir.steps.len(), 1);
        let call = ir.steps[0].as_call().expect("flat call step");
        assert_eq!(call.ability.as_str(), "chat");
        assert_eq!(
            call.target,
            IrTarget::Device {
                node_id: "node-1".to_string()
            },
            "traditional `call ... on ...` must lower to IrTarget::Device"
        );
    }

    #[test]
    fn member_call_dispatches_to_agent_via_recorder() {
        // The interpreter dispatch path receives the resolved
        // IrTarget::Agent — no string-based classification along the way.
        use crate::core::agent::id::AgentId;

        let src = r#"
            mission "member-call" {
                let r = claude.chat(prompt: "hi")
            }
        "#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        let dispatcher = ShapeRecordingDispatcher::new();
        execute_with_dispatcher(&dispatcher, "tenant", &ir).unwrap();

        let seen = dispatcher.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].0,
            IrTarget::Agent(AgentId::parse("claude").unwrap())
        );
        assert_eq!(seen[0].1.as_str(), "chat");
        assert_eq!(seen[0].2.get("prompt").and_then(|v| v.as_str()), Some("hi"));
    }

    #[test]
    fn traditional_call_dispatches_to_device_via_recorder() {
        let src = r#"
            mission "traditional" {
                let r = call "chat" on "node-1" with { prompt = "hi" }
            }
        "#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        let dispatcher = ShapeRecordingDispatcher::new();
        execute_with_dispatcher(&dispatcher, "tenant", &ir).unwrap();

        let seen = dispatcher.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].0,
            IrTarget::Device {
                node_id: "node-1".to_string()
            }
        );
        assert_eq!(seen[0].1.as_str(), "chat");
    }

    // ── PR-10 Stage 3: loop executor audit hooks ──────────────────────────
    //
    // These cover the RFC §4 / §5 behaviours the planner tests
    // cannot: runtime termination, typed error surfaces, depth
    // non-nesting, and the `<name>.result` export contract.

    /// Programmable dispatcher: returns canned JSON Values per call,
    /// indexed by a per-ability counter so tests can script the
    /// sequence of verify outputs across iterations.
    struct ScriptedDispatcher {
        outputs: Arc<Mutex<std::collections::HashMap<String, Vec<Value>>>>,
        cursor: Arc<Mutex<std::collections::HashMap<String, usize>>>,
        default: Value,
        calls: Arc<Mutex<Vec<String>>>,
        /// If set, each dispatch reads `EASYNET_AGENT_DEPTH` and
        /// records the observed depth value. Used by the
        /// non-nesting-depth test.
        depth_observations: Arc<Mutex<Vec<Option<String>>>>,
    }

    impl ScriptedDispatcher {
        fn new(default: Value) -> Self {
            Self {
                outputs: Arc::new(Mutex::new(std::collections::HashMap::new())),
                cursor: Arc::new(Mutex::new(std::collections::HashMap::new())),
                default,
                calls: Arc::new(Mutex::new(Vec::new())),
                depth_observations: Arc::new(Mutex::new(Vec::new())),
            }
        }
        fn with_script(self, ability: &str, script: Vec<Value>) -> Self {
            self.outputs
                .lock()
                .unwrap()
                .insert(ability.to_string(), script);
            self
        }
    }

    impl StepDispatcher for ScriptedDispatcher {
        fn dispatch(
            &self,
            _run: RunContext<'_>,
            _target: &IrTarget,
            ability: &AbilityName,
            _arguments: &Value,
            _timeout_ms: Option<u64>,
            _dependency_receipts: &[ChildInvocationReceiptAnchor],
        ) -> Result<StepDispatchOutcome, EalError> {
            let k = ability.as_str().to_string();
            self.calls.lock().unwrap().push(k.clone());
            self.depth_observations
                .lock()
                .unwrap()
                .push(std::env::var("EASYNET_AGENT_DEPTH").ok());
            let mut cursors = self.cursor.lock().unwrap();
            let cur = cursors.entry(k.clone()).or_insert(0);
            let outs = self.outputs.lock().unwrap();
            if let Some(script) = outs.get(&k) {
                if *cur < script.len() {
                    let v = script[*cur].clone();
                    *cur += 1;
                    return Ok(v.into());
                }
            }
            Ok(self.default.clone().into())
        }
        fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
            // Loops are sequential by design — no thread cloning needed
            // for these tests. Signal "fall back to sequential" via Err.
            Err(EalError::Internal(
                "scripted dispatcher is single-thread".into(),
            ))
        }
    }

    /// RFC §4.4 happy path: verify returns `done: true` on iter K;
    /// the loop terminates successfully, and `<name>.result` binds
    /// the verify final call's output.
    #[test]
    fn loop_terminates_on_done_true_and_binds_result() {
        let src = r#"
            mission "t" {
                loop "review" max_iters: 4 {
                    body { a.step(p: "x") }
                    verify { a.ok(p: "x") }
                }
            }"#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        // Verify script: iter 1 → done:false; iter 2 → done:true.
        let d = ScriptedDispatcher::new(serde_json::json!({"done": false})).with_script(
            "ok",
            vec![
                serde_json::json!({"done": false}),
                serde_json::json!({"done": true, "payload": "winner"}),
            ],
        );
        let report = execute_with_dispatcher(&d, "test", &ir).unwrap();
        assert_eq!(report.steps_failed, 0);
        // Body + verify × 2 iterations = 4 calls total.
        assert_eq!(d.calls.lock().unwrap().len(), 4);
        // `<name>.result` export: captured as review.result, verify
        // final output on the winning iteration.
        let winner = report
            .outputs
            .get("review.result")
            .expect("review.result must be exported on winning iter");
        assert!(
            winner.contains("winner"),
            "result must carry verify final output; got: {winner}"
        );
    }

    /// Loop-internal joins retain dependency receipts from the
    /// iteration scope and receive the mission run's `RunContext`
    /// (trace_id = mission_id).
    #[test]
    fn loop_steps_retain_dependency_receipts_and_trace_id() {
        type RecordedCalls = Arc<Mutex<Vec<(String, String, Vec<ChildInvocationReceiptAnchor>)>>>;

        struct RecordingDispatcher {
            // (ability, trace_id, dependency receipts) per dispatch.
            calls: RecordedCalls,
        }
        impl StepDispatcher for RecordingDispatcher {
            fn dispatch(
                &self,
                run: RunContext<'_>,
                _target: &IrTarget,
                ability: &AbilityName,
                _arguments: &Value,
                _timeout_ms: Option<u64>,
                dependency_receipts: &[ChildInvocationReceiptAnchor],
            ) -> Result<StepDispatchOutcome, EalError> {
                let qualified = format!("a.{}", ability.as_str());
                self.calls.lock().unwrap().push((
                    ability.as_str().to_string(),
                    run.trace_id.to_string(),
                    dependency_receipts.to_vec(),
                ));
                Ok(StepDispatchOutcome {
                    value: serde_json::json!({"done": true}),
                    invocation: ChildInvocationRecord::for_test(&qualified, 0xab),
                })
            }
            fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
                Err(EalError::Internal("single-thread".into()))
            }
        }

        let src = r#"
            mission "t" {
                loop "review" max_iters: 2 {
                    body { let draft = a.step(p: "x") }
                    verify { a.ok(doc: draft.output) }
                }
            }"#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let d = RecordingDispatcher {
            calls: Arc::clone(&calls),
        };
        let report = execute_with_dispatcher(&d, "test", &ir).unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "one iteration: body + verify");
        for (ability, trace_id, _) in calls.iter() {
            assert_eq!(
                trace_id, &report.trace.mission_id,
                "{ability}: every dispatch must carry the run's trace id"
            );
            assert!(!trace_id.is_empty());
        }
        let (_, _, step_parents) = &calls[0];
        assert!(
            step_parents.is_empty(),
            "iteration root must have no dependency receipts"
        );
        let (_, _, ok_parents) = &calls[1];
        assert_eq!(
            ok_parents.len(),
            1,
            "verify step must retain the body step receipt"
        );
        assert!(ok_parents[0]
            .projection()
            .get("receipt_ura")
            .and_then(Value::as_str)
            .is_some());
    }

    #[test]
    fn fan_in_join_retains_each_producer_receipt() {
        type RecordedCalls = Arc<Mutex<Vec<(String, Vec<ChildInvocationReceiptAnchor>)>>>;

        struct JoinRecordingDispatcher {
            calls: RecordedCalls,
            next_marker: Arc<AtomicU32>,
        }

        impl StepDispatcher for JoinRecordingDispatcher {
            fn dispatch(
                &self,
                _run: RunContext<'_>,
                _target: &IrTarget,
                ability: &AbilityName,
                _arguments: &Value,
                _timeout_ms: Option<u64>,
                dependency_receipts: &[ChildInvocationReceiptAnchor],
            ) -> Result<StepDispatchOutcome, EalError> {
                self.calls
                    .lock()
                    .unwrap()
                    .push((ability.as_str().to_string(), dependency_receipts.to_vec()));
                let marker = self.next_marker.fetch_add(1, Ordering::SeqCst) as u8;
                Ok(StepDispatchOutcome {
                    value: serde_json::json!({"ability": ability.as_str()}),
                    invocation: ChildInvocationRecord::for_test(ability.as_str(), marker),
                })
            }

            fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
                Ok(Box::new(Self {
                    calls: Arc::clone(&self.calls),
                    next_marker: Arc::clone(&self.next_marker),
                }))
            }
        }

        let source = r#"
            mission "fan-in" {
                let left = call "produce.left" on "local"
                let right = call "produce.right" on "local"
                let joined = call "join" on "local" with {
                    left = left.output,
                    right = right.output
                }
            }
        "#;
        let ir = planner::compile(&parser::parse(source).unwrap()).unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let dispatcher = JoinRecordingDispatcher {
            calls: Arc::clone(&calls),
            next_marker: Arc::new(AtomicU32::new(1)),
        };

        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();
        assert_eq!(report.steps_failed, 0);

        let calls = calls.lock().unwrap();
        let (_, join_receipts) = calls
            .iter()
            .find(|(ability, _)| ability == "join")
            .expect("join step dispatched");
        assert_eq!(join_receipts.len(), 2);
        let receipt_uras = join_receipts
            .iter()
            .map(|receipt| {
                receipt
                    .projection()
                    .get("receipt_ura")
                    .and_then(Value::as_str)
                    .expect("typed receipt reference has canonical URA")
                    .to_string()
            })
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(receipt_uras.len(), 2, "join must retain both producers");
    }

    /// `done: bool` must be found through the daemon shell-executor
    /// envelope: lowered verify steps capture `{result: "<stdout
    /// json>", fulfilled_by, ...}`, with the handler payload nested
    /// as a JSON string.
    #[test]
    fn verify_done_peels_shell_envelope() {
        let envelope = serde_json::json!({
            "result": "{\"op\":\"eval.loop_gate\",\"done\":true}",
            "fulfilled_by": "device/loopback",
            "exit_code": 0,
        });
        let bytes = serde_json::to_vec(&envelope).unwrap();
        assert!(matches!(verify_output_done(&bytes), VerifyDone::True));

        // Bare payloads (in-process dispatch) still work.
        let bare = serde_json::to_vec(&serde_json::json!({"done": false})).unwrap();
        assert!(matches!(verify_output_done(&bare), VerifyDone::False));
    }

    /// A named loop's `<name>.result` export feeds a downstream step
    /// (`.result` accessor): the downstream step retains the winning
    /// iteration's verified terminal receipt, and the run-level
    /// `__runner_receipt_graph__` substitution carries loop-internal
    /// invocation records.
    #[test]
    fn loop_result_feeds_downstream_step_with_receipt_chain() {
        type RecordedCalls = Arc<Mutex<Vec<(String, Value, Vec<ChildInvocationReceiptAnchor>)>>>;

        struct ArgRecordingDispatcher {
            // (ability, arguments, dependency receipts) per dispatch.
            calls: RecordedCalls,
        }
        impl StepDispatcher for ArgRecordingDispatcher {
            fn dispatch(
                &self,
                _run: RunContext<'_>,
                _target: &IrTarget,
                ability: &AbilityName,
                arguments: &Value,
                _timeout_ms: Option<u64>,
                dependency_receipts: &[ChildInvocationReceiptAnchor],
            ) -> Result<StepDispatchOutcome, EalError> {
                let qualified = format!("a.{}", ability.as_str());
                self.calls.lock().unwrap().push((
                    ability.as_str().to_string(),
                    arguments.clone(),
                    dependency_receipts.to_vec(),
                ));
                Ok(StepDispatchOutcome {
                    value: serde_json::json!({"done": true}),
                    invocation: ChildInvocationRecord::for_test(&qualified, 0xab),
                })
            }
            fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
                Err(EalError::Internal("single-thread".into()))
            }
        }

        let src = r#"
            mission "t" {
                loop "refine" max_iters: 2 {
                    body { let draft = a.step(p: "x") }
                    verify { let gated = a.ok(doc: draft.output) }
                }
                let published = a.publish(
                    doc: refine.result,
                    receipt_graph: "__runner_receipt_graph__"
                )
            }"#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let d = ArgRecordingDispatcher {
            calls: Arc::clone(&calls),
        };
        let report = execute_with_dispatcher(&d, "test", &ir).unwrap();
        assert_eq!(report.steps_failed, 0);

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 3, "body + verify + downstream publish");
        let (ability, publish_args, publish_parents) = &calls[2];
        assert_eq!(ability, "publish");
        // Loop-boundary receipt edge: the publish step's parent is the
        // winning iteration's final verify invocation.
        assert_eq!(publish_parents.len(), 1);
        assert!(publish_parents[0]
            .projection()
            .get("receipt_ura")
            .and_then(Value::as_str)
            .is_some());
        // `doc:` resolved from the loop's exported result payload.
        assert_eq!(
            publish_args.get("doc").and_then(|d| d.get("done")),
            Some(&Value::Bool(true))
        );
        // Run-level receipt graph: loop-internal records included.
        let graph = publish_args
            .get("receipt_graph")
            .and_then(Value::as_array)
            .expect("sentinel must substitute to an array");
        let nodes: Vec<_> = graph
            .iter()
            .filter_map(|e| e.get("ability").and_then(Value::as_str))
            .collect();
        assert_eq!(nodes, vec!["a.step", "a.ok"]);
    }

    /// RFC §5.2: `LoopExhausted` — max_iters reached without
    /// done:true. Mission outcome is Aborted; error surface cites
    /// "LoopExhausted" and "max_iters".
    #[test]
    fn loop_exhausts_with_typed_error() {
        let src = r#"
            mission "t" {
                loop "x" max_iters: 3 {
                    body { a.step(p: "x") }
                    verify { a.ok(p: "x") }
                }
            }"#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        // Never done.
        let d = ScriptedDispatcher::new(serde_json::json!({"done": false}));
        let report = execute_with_dispatcher(&d, "test", &ir).unwrap();
        assert_eq!(report.trace.outcome, MissionOutcome::Aborted);
        assert!(report.steps_failed >= 1);
        // 3 iters × (body + verify) = 6 calls.
        assert_eq!(d.calls.lock().unwrap().len(), 6);
    }

    /// RFC §4.4 / §5.2: `VerifyMalformed` — verify output missing
    /// `done` field. Mission aborts at iter 1.
    #[test]
    fn verify_without_done_field_aborts() {
        let src = r#"
            mission "t" {
                loop max_iters: 3 {
                    body { a.step(p: "x") }
                    verify { a.ok(p: "x") }
                }
            }"#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        // Verify returns an object with NO `done` field.
        let d = ScriptedDispatcher::new(serde_json::json!({"ok": true}));
        let report = execute_with_dispatcher(&d, "test", &ir).unwrap();
        assert_eq!(report.trace.outcome, MissionOutcome::Aborted);
        // Stopped at iter 1 (body + verify).
        assert_eq!(d.calls.lock().unwrap().len(), 2);
    }

    /// RFC §4.4 / §5.2: non-boolean `done` is also VerifyMalformed.
    /// Pins the strict-bool contract — a string "true" does NOT
    /// count, to stop authors from shipping prose-predicate verify.
    #[test]
    fn verify_with_non_bool_done_aborts() {
        let src = r#"
            mission "t" {
                loop max_iters: 2 {
                    body { a.step(p: "x") }
                    verify { a.ok(p: "x") }
                }
            }"#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        let d = ScriptedDispatcher::new(serde_json::json!({"done": "yes"}));
        let report = execute_with_dispatcher(&d, "test", &ir).unwrap();
        assert_eq!(report.trace.outcome, MissionOutcome::Aborted);
    }

    /// RFC §4.2 / §4.3: loop iterations do NOT stack agent-dispatch
    /// depth. A 4-iter loop dispatching once per body and once per
    /// verify runs 8 total dispatches, but each stays at the same
    /// `EASYNET_AGENT_DEPTH` value the mission was invoked at — it
    /// does not climb with iteration count. The dispatcher records
    /// the env var each call; all values must be equal.
    #[test]
    fn loop_with_body_dispatch_does_not_nest_depth() {
        let src = r#"
            mission "t" {
                loop max_iters: 4 {
                    body { a.step(p: "x") }
                    verify { a.ok(p: "x") }
                }
            }"#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        let d = ScriptedDispatcher::new(serde_json::json!({"done": false}));
        let _ = execute_with_dispatcher(&d, "test", &ir).unwrap();
        let obs = d.depth_observations.lock().unwrap();
        // 4 iters * (body + verify) = 8 dispatches.
        assert_eq!(obs.len(), 8);
        // Every observation is identical — no climbing depth.
        let first = obs.first().cloned().flatten();
        for (i, v) in obs.iter().enumerate() {
            assert_eq!(
                v.clone(),
                first.clone(),
                "iter-dispatch {i} observed depth {v:?}, differs from first {first:?}"
            );
        }
    }
}
