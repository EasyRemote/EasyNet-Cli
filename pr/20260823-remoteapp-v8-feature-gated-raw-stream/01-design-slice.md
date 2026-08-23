# RemoteApp v8 raw stream feature gate

## Product seam

RemoteApp and EasyRemote need ABI v8 raw stream payloads for high-frequency
desktop/window media. The SDK adapters selected v8 only from exported symbol
presence. In libeasynet_cli the v8 symbol can exist while the runtime feature is
disabled, because `runtime_feature_discovery().abi_extensions.v8.stream_raw_payload`
is the authoritative capability bit. Symbol-only selection can therefore route
RemoteApp media into a raw path that the runtime cannot execute, instead of
falling back to the canonical v7 JSON/base64 stream representation.

## Invariants

- ABI v8 raw stream is used only when both the v8 symbol exists and feature
  discovery advertises `abi_extensions.v8.stream_raw_payload == true`.
- When the feature bit is absent, malformed, or false, SDKs use v7 stream open.
- Feature discovery parsing must be fail-closed for the v8 extension: unknown
  or invalid extension shape never upgrades to raw bytes.
- The v8 metadata state machine remains unchanged; raw bytes are still only a
  transport representation.

## Expected impact

This makes RemoteApp media path selection deterministic across runtime builds.
A v7-only or axon-pb-disabled runtime remains usable through v7 fallback, while
true v8 runtimes continue to deliver raw payload bytes without base64 overhead.

## Verification

- Failed first from repository root because Go tests must run inside the Go
  module: `go test . -run 'TestRawStreamPacket' && go test -tags runtime_cabi . -run 'TestCABIRuntimeProvider(...)'`.
- Failed before fixture update, proving the new branch exercised
  feature-disabled selection: `go test -tags runtime_cabi . -run 'TestCABIRuntimeProvider(...)'`.
- Passed: `go test .` from `sdk/go`.
- Passed: `go test -tags runtime_cabi . -run 'TestCABIRuntimeProvider(DispatchesStreamBeforeTerminal|FallsBackToV7StreamOpen|FallsBackToV7WhenV8FeatureDisabled|PreservesStreamOrderAndSingleTerminal|RejectsCallbackBackpressure|MemoizesConcurrentStreamCancellation)'`.
- Passed: `uv run --project sdk/python pytest -q sdk/python/tests/test_cabi.py`.
