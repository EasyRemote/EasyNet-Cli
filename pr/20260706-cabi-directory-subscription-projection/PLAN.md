# C ABI Directory Subscription Projection Plan

## Objective

Close the C ABI Directory subscribe projection gap without moving protocol
truth into language SDKs.

Go and Python now prove `directory/subscription_stream` over shared fixtures.
The remaining lower-layer gap is that the C ABI Directory profile exposes
read-model list/resolve carriers but still lacks Directory-owned
`directory.subscribe` carrier and `DirectorySubscription` projection symbols.

## Invariants

- The SPEC remains unchanged.
- Directory profile owns `directory.subscribe` carrier/projection; Events
  continues to own `federation.subscribe_directory_v2` event-frame projection.
- C ABI must delegate carrier/projection semantics to
  `protocol::directory_contract`; language SDKs must not synthesize descriptor
  refs or subscription DTO state.
- The carrier must preserve caller, callee, subject, nonce, causal_context,
  descriptor_version, args, and metadata.
- Subscription projection must keep cursor, resume_token, drop_count, bounded
  events, and snapshot-before-live state validation.

## Implementation Steps

1. Add shared Directory subscription carrier and projection functions to
   `src/protocol/directory_contract.rs`.
2. Export C ABI symbols in `src/ffi/directory/mod.rs` and
   `include/easynet_cli.h`.
3. Bind the new symbols in Python C ABI v4 transport.
4. Update Python C ABI tests to prove Directory profile symbols are used for
   subscribe carriers/projection instead of Events profile carriers.
5. Update parity/scaffold evidence and run deterministic verification.

## Boundary Proof

The SDK remains a facade: C ABI builds a daemon `directory.subscribe`
Invocation and projects a daemon stream-open/subscription DTO. It does not
execute directory fan-out, own a directory database, or convert
`federation.subscribe_directory_v2` event frames into Directory profile facts.
Runtime Core remains the submit/open path.

## Verification Plan

- `cargo test ffi::directory`
- `uv run python -m unittest tests.test_cabi.CABITransportTests.test_directory_subscribe_opens_runtime_stream`
- `uv run python -m unittest discover tests`
- `go test ./...`
- `cargo test --bin sdk-conformance-runner`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Decisions Log

- C ABI is now declared as required evidence for
  `directory/subscription_stream`; the adapter report points to
  `src/ffi/directory/mod.rs` tests.
- Feature discovery reports `directory_subscription_projection` and upgrades
  the Directory + Identity profile status to
  `read_model_subscription_projection_partial`.
- Directory profile owns `directory.subscribe` subscription state; Events
  remains the owner of `federation.subscribe_directory_v2` event frames.

## Verification Result

- PASS: `cargo test ffi::directory --lib`
- PASS: `cargo test directory_contract --lib`
- PASS: `uv run python -m unittest discover -s tests -p 'test_cabi.py' -k directory_project_subscription`
- PASS: `uv run python -m unittest discover -s tests -p 'test_cabi.py' -k directory_subscribe`
- PASS: `uv run python -m unittest discover tests`
- PASS: `go test ./...`
- PASS: `cargo test --bin sdk-conformance-runner`
- PASS: `bash tools/scripts/check-sdk-scaffold.sh`
- PASS: `cargo fmt --check`
- PASS: `python -m compileall easynet_sdk`
- PASS: `git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md` produced no diff.
