# Python SDK Environment Default Control Path

## Objective

Make `SdkEnvironment` own default daemon control-path resolution for Python SDK client factories before any C ABI, direct runtime, Invocation, or profile transport is opened.

## Boundary Proof

- This slice is process-root SDK configuration only.
- It does not implement URA, DescriptorRef, receipt verification, Invocation signing, or host-stream protocol semantics in Python.
- Explicit `ConnectOptions.control_path` must override the environment path.
- Explicit `SdkEnvironment.control_path` must override the SDK default path.
- Empty environment paths must resolve through `control_ipc.default_control_path()` before reaching runtime/profile transports.

## Implementation Tasks

- Add a public `SdkEnvironment.resolved_control_path()` and private `ConnectOptions` normalization helper.
- Route all environment-owned daemon/profile/client factories through that resolver.
- Preserve feature discovery as library-only, with no daemon control discovery dependency.
- Add focused tests for default path propagation and explicit option override behavior.
- Update Python SDK docs and parity notes.

## Verification

- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_environment.py -q`
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests -q`
- `ruff check sdk/python`
- `bash tools/scripts/check-sdk-scaffold.sh`
