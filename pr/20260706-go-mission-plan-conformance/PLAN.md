# Go MissionPlan Child Invocation Conformance Plan

## Objective

Converge the Go Mission facade with Python by adding an SDK-owned MissionPlan
object model that renders EAL and validates daemon-observed child Invocation
facts against declared child intents.

## Invariants

- The SPEC remains unchanged.
- Mission/EAL remains daemon orchestration; this slice does not execute EAL,
  schedule retries, or create child Invocations locally.
- The SDK may render Mission source and validate observed daemon facts, but it
  must not fabricate receipt refs or mark unanchored children as receipt-backed.
- Go and Python must expose the same concepts: MissionPlan, steps, child
  invocation intents, and child invocation conformance.
- Validation must fail closed with typed mission profile details when expected
  child steps are missing, unexpected, or ability-mismatched.

## Implementation Steps

1. Add Go MissionPlan, MissionPlanStep, step output reference, child intent, and
   conformance DTOs to `sdk/go/mission.go`.
2. Add Go tests matching the existing Python MissionPlan behavior.
3. Extend shared Mission conformance expectations and Go conformance tests to
   require MissionPlan child fact validation.
4. Update parity notes if needed to reflect real Go evidence rather than a doc
   claim.
5. Run targeted Go/Python Mission tests plus Rust Mission contract and hygiene.

## Boundary Proof

MissionPlan is a facade object for source generation and post-run evidence
checking. It does not own daemon transport, EAL execution, retry scheduling, or
receipt verification. Child Invocations remain daemon/Axon facts surfaced in
MissionStatus.

## Verification Plan

- `go test ./... -run 'Mission|Conformance'`
- `uv run python -m unittest tests.test_mission tests.test_conformance`
- `cargo test mission_contract --lib`
- `cargo fmt --check`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Verification Result

- PASS: `go test ./... -run 'Mission|Conformance'`
- PASS: `uv run python -m unittest tests.test_mission tests.test_conformance`
- PASS: `cargo test mission_contract --lib`
- PASS: `cargo fmt --check`
- PASS: `git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Completed Scope

- Added Go MissionPlan source rendering, deterministic step aliases, scalar
  field validation, in-plan step output references, child Invocation intents,
  and daemon MissionStatus fact conformance.
- Added shared MissionPlan child Invocation conformance case for Go and Python.
- Connected Go shared Mission events fixture validation to the existing carrier
  status conformance case.
- Updated parity evidence to distinguish completed SDK plan/fact conformance
  from still-incomplete daemon execution, scheduler, and live stream adapters.
