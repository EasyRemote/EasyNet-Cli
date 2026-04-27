# Test-suite flake localization — 2026-04-27

## TL;DR

The intermittent `cargo test --lib` failures across the recent
commits are **not** flake — they are a deterministic + concurrency
bug in `runtime::agents::fleet_lifecycle_ability::tests`. Class:
process-global `EASYNET_HOME` env-var mutation under a parallel test
runner. Same family as the `P4.10` HomeGuard fix from earlier work
(33d478a); that fix did not propagate here.

## Evidence

### Determinism class

`cargo test --lib -- --test-threads=1` (single-threaded):
```
test result: FAILED. 846 passed; 2 failed
  runtime::agents::fleet_lifecycle_ability::tests::start_agent_persists_and_returns_canonical_uri
  runtime::agents::fleet_lifecycle_ability::tests::stop_agent_by_uri_extracts_name_tail
```
Two tests fail deterministically, even alone. They are wrong tests
(or wrong handlers); not a race.

### Concurrency class

`cargo test --lib` (parallel) — five back-to-back full runs:
```
run 1: 4 failed
run 2: 3 failed
run 3: 4 failed
run 4: 4 failed
run 5: 4 failed
```
Failure count varies 2–4. The two extra failers that come and go are
also in `fleet_lifecycle_ability::tests`:
- `start_agent_replaces_existing_row_and_signals_replaced_prior`
- `stop_agent_by_name_acks_true_and_removes_row`

### Cause

`fleet_lifecycle_ability::tests` defines a `HomeGuard` fixture
(`fleet_lifecycle_ability.rs:239`) that does:

```rust
std::env::set_var("EASYNET_HOME", &path);
```

`std::env::set_var` is **process-global**. Two tests calling
`HomeGuard::new()` in parallel both mutate one env var; whoever
writes second wins, and the first test's filesystem operations
(load_agents, save) land in the loser's tempdir. The first test
then asserts on a directory that its own writes never reached.

The Drop impl (`fleet_lifecycle_ability.rs:260`) restores the prior
value, which makes the corruption window narrow but doesn't close
it: any operation between the racing tests' `set_var` calls and
their respective filesystem touches will see the wrong HOME.

### Why my modules are clean

`cargo test --lib -- runtime::agents::{mcp_bridge,a2a_bridge,
fleet_list_agents,meta,network_health,policy,permission}_ability
registry::a2a_labels runtime::agents::tests` — 50 tests, ran 5×
serial and 10× parallel:

```
15/15 runs: 50 passed; 0 failed
```

None of the abilities I shipped touch process-global env state. The
audit invariant held on the surface I added.

## Fix shape (for whoever owns fleet_lifecycle_ability)

Two paths, in order of preference:

1. **Replace the env-var fixture with a thread-local override.**
   Have the production code's HOME resolver consult a `tokio::task_local!`
   or `thread_local!` first, env-var second. Tests set the
   thread-local; production sets the env var. No racing because
   thread-locals are per-test-thread by definition.

2. **Serialize the fleet_lifecycle_ability tests.** Wrap the test
   bodies in a `static MUTEX: Mutex<()> = Mutex::new(())` and lock
   in each test. This is the cheap fix; it documents that these
   tests cannot run in parallel without lying about their isolation.

The two deterministic failers (`start_agent_persists_and_returns…`
and `stop_agent_by_uri_extracts_name_tail`) are independent bugs in
the handler logic — they fail even alone. Those need their own fix:
the assertion lines are 292 and 375 in `fleet_lifecycle_ability.rs`.

## Audit-grade implication

Per the user's review, "any nondeterminism in tests is a signal."
The signal here is: a test fixture that mutates global state is
incompatible with the parallel test runner. The fix above
(thread-local fallback) preserves the audit story; the cheap fix
(serial mutex) trades a bit of test wallclock for the same property.

Either is acceptable. Doing nothing means future PRs in this area
will land on a flaky baseline, and a real bug elsewhere will be
masked by "oh, it was probably the fleet_lifecycle thing."
