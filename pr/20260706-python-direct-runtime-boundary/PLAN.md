# Python Direct Runtime Boundary

## Objective

Keep Python direct daemon UDS/gRPC runtime transport as an SDK-internal provider
while preventing EasyRemote and root-package consumers from depending on that
transport or Axon protobuf modules directly. Product code should compose
through public Runtime Core facade objects, not SDK-internal daemon transport
classes.

## Boundary Proof

- Axon owns protobuf wire semantics and Invocation canonical rules.
- EasyNet-Cli SDK may use generated Axon protobufs internally to implement a
  daemon provider.
- EasyRemote must consume public SDK facades only; it must not import
  `easynet_sdk.direct_runtime`, direct daemon transport classes, or protobuf
  module aliases.

## Invariants

- `DirectDaemonRuntimeConnector` and `DirectDaemonRuntimeTransport` remain
  importable from `easynet_sdk.direct_runtime` for SDK-internal tests and
  provider construction, but they are not exported from `easynet_sdk`.
- `easynet_sdk.direct_runtime` does not publish public `invoke_pb2`,
  `invoke_pb2_grpc`, or `types_pb2` aliases.
- Consumer boundary auditing rejects EasyRemote references to direct runtime
  internals while allowing public facade transports such as
  `DaemonInvocationTransport`.
- Direct runtime behavior tests still pass, proving the internal provider path
  was not removed or stubbed.

## Implementation Plan

1. Remove root package exports for direct runtime transport internals.
2. Keep protobuf module references private inside `direct_runtime.py`.
3. Extend consumer-boundary auditing and shared conformance expectations for
   direct runtime internal imports.
4. Run import-boundary, direct-runtime, conformance, EasyRemote boundary, and
   scaffold checks before committing.

## Remaining Outside This Slice

- Backend raw Axon/protobuf/direct daemon transport migration.
- Live daemon keyring policy and product pairing lifecycle cutover.
- Non-P0 language facades.

## Verification Results

- `PYTHONPATH=tests ./.venv/bin/python -m unittest tests.test_import_boundary tests.test_direct_runtime tests.test_conformance`
  passed in `sdk/python`.
- `bash tools/scripts/check-easyremote-sdk-boundary.sh --self-test` passed.
- `bash tools/scripts/check-easyremote-sdk-boundary.sh /Users/macbook.silan.tech/Documents/GitHub/EasyRemote`
  passed.
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format jsonl`
  passed.
- `bash tools/scripts/check-sdk-scaffold.sh` passed.
- `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test` passed.
- `git diff --check` passed.
