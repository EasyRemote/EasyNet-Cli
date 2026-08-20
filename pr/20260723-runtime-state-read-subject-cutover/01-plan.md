# Runtime-state read subject cutover

## Goal

Remove the remaining implicit daemon-self subject default from operator/runtime-state read surfaces that expose product state such as status, ability catalog, and invocation history. These reads must enter LocalRuntime with an explicit system subject policy rather than relying on transport-layer callee/self fallback.

## Root abstraction problem

`invoke_local_ability(name, args)` is still a convenient product ingress that lowers to `LocalDaemonLoopbackSubjectPolicy::LocalDaemonSelf`. That policy is acceptable only as an internal daemon-system shortcut when the caller has deliberately selected daemon-self semantics. It is not acceptable for product/runtime-state reads because the product surface becomes unable to prove which semantic subject it selected before route/admission.

The failure mode is visible in user-facing errors where history/catalog/browser discovery reaches authority or descriptor resolution with mismatched session subject, envelope subject, and caller signer ownership. The SDK is correctly fail-closing; product ingress must stop defaulting tuple facts.

## Invariants

1. Runtime-state reads must choose a subject before transport entry.
2. Missing or malformed identity state must fail as identity/authority readiness, not as route/catalog absence.
3. CLI public behavior remains source-compatible: existing commands and output shapes stay stable when identity state is valid.
4. No new fallback path may synthesize caller, callee, or subject from stale directory rows.
5. Boundary gates must make regression detectable without product e2e timing.

## Boundary proof

- `LocalDaemonSystemAbilityIssuer` already exists as the named product-system issuer for explicit subjects.
- Runtime-state CLI wrappers should use that issuer with a deterministic local daemon/device subject.
- Raw public tuple entry points (`easynet invoke`, stream, bidi) already require explicit subject/nonce and are not changed here.
- The generic `invoke_local_ability` convenience is not removed in this iteration because its call surface is broad; this iteration narrows the highest-risk runtime-state read ingress first and adds a gate to prevent the seam from returning there.

## Verification plan

1. Add unit coverage for the selected subject policy.
2. Add a shell boundary gate that forbids `invoke_local_ability` in runtime-state read modules.
3. Wire the gate into the canonical convergence script.
4. Run targeted tests, formatting, architecture gates, and codegraph sync.

## Decisions

- Treat status/ability catalog/invocation history as runtime-state reads.
- Use explicit daemon-local system subject rather than user/session authority subject for local runtime-state reads. User/session authority remains a higher-level product authorization concern and must not be silently projected into device-owned runtime reads.
