# SDK Runtime Lifecycle Neutrality

Date: 2026-07-24

## Goal

Converge the Go SDK runtime lifecycle facade toward the shared canonical
runtime model by removing product/daemon lifecycle vocabulary from the SDK core
surface. The EasyNet provider package may still bind the canonical lifecycle to
`easynet-daemon`; the canonical SDK root must describe only runtime hosts,
runtime lifecycle state, transports, endpoints, and handles.

## Root abstraction problem

`sdk/go/runtime_lifecycle.go` owns the provider-neutral `RuntimeHost` facade, but
its diagnostics and comments still name daemon discovery/control/handles. That
keeps product host semantics inside the canonical runtime model and diverges
from the Python SDK lifecycle facade, which already reports runtime-host
semantics.

## Boundary proof

- Canonical SDK root may expose:
  - runtime host discovery/start/attach/status/stop;
  - runtime lifecycle transport seams;
  - runtime endpoint/status decoding;
  - runtime handle attachment state.
- Canonical SDK root must not expose:
  - daemon discovery/control/handle terminology;
  - product process ownership vocabulary;
  - EasyNet-specific lifecycle policy.
- EasyNet-specific daemon binding remains in:
  - `sdk/go/provider/easynet/*`;
  - native C ABI transport implementation names that bind to CLI artifacts.

## Invariants

1. Public Go SDK types and method names remain compatible.
2. Error codes, stages, retry hints, and retryability remain compatible.
3. Go and Python lifecycle diagnostics use the same canonical runtime-host
   vocabulary.
4. The v2 architecture gate fails if daemon lifecycle vocabulary re-enters
   `sdk/go/runtime_lifecycle.go`.
5. Provider/easynet packages are not constrained by this root SDK gate.

## Verification plan

- Run Go SDK tests after refactor.
- Run product-neutrality and canonical runtime convergence gates.
- Run architecture convergence gate and self-test.
- Run formatter/diff checks.
- Use codegraph after the edit to verify the remaining lifecycle seam points to
  provider binding rather than canonical core daemon vocabulary.

