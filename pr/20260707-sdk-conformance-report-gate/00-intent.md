# SDK Conformance Report Gate

## Objective

Tighten daemon SDK cutover readiness by making language action-adapter reports
part of the aggregate gate.

The target architecture remains:

```text
Axon protocol truth -> EasyNet-Cli daemon/Rust/C ABI -> language SDK facades
```

This slice does not add protocol behavior. It prevents status drift: any
provider-backed or shipped seam claim must keep its shared conformance
action-report closed over the manifest, backed by repository-local evidence,
and executable through `sdk-conformance-runner`.

## Scope

1. Add a repository script that validates Rust, C ABI, Go, Python, and Node
   action-adapter reports through `sdk-conformance-runner`.
2. Add a self-test proving the gate fails when a required report record is
   missing.
3. Wire the report gate into `check-sdk-cutover-readiness.sh`.
4. Register the script in the SDK scaffold check so the gate cannot disappear.

## Non-goals

- Do not edit `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not move product cutover work into the SDK parity matrix.
- Do not change language SDK semantics or add compatibility fallback behavior.
