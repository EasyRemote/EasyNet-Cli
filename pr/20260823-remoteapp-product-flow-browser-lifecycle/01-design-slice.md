# RemoteApp product-flow Browser/Tauri lifecycle gate

## Problem

`frontend-remoteapp-product-flow-e2e.sh` previously bundled frontend typecheck,
component coverage, Hub/runtime readiness, and host probes, but it did not require
the real Browser/Tauri RemoteApp lifecycle verifier in its `--run` path.

That left a product seam: the flow could be used as an E2E entrypoint while still
missing evidence that the user-facing surface can execute the actual lifecycle:
target picker, permission preflight, consent, session creation, attach, event
watch, media visibility, input/control policy, terminal receipt, and session end.

## Architecture boundary

The product-flow harness is an evidence aggregator, not the owner of RemoteApp
semantics. Browser/Tauri lifecycle semantics remain owned by
`frontend-remoteapp-browser-lifecycle-e2e.sh`; host capture/control semantics
remain owned by the host RemoteApp probes.

The product-flow harness now only composes those gates in product order:

1. Hub API readiness
2. daemon runtime readiness
3. frontend static/type/UI checks
4. real Browser/Tauri lifecycle evidence
5. host permission/capture/input probes

## Invariants

- `--run` must fail closed unless Browser/Tauri lifecycle evidence is supplied
  through an evidence JSON artifact or an explicit runner command.
- Component tests are not accepted as a substitute for Browser/Tauri lifecycle
  evidence.
- Browser/Tauri lifecycle evidence must run after frontend UI coverage and before
  host-only probes, so the product bundle cannot hide frontend lifecycle gaps
  behind host evidence.
- This slice does not claim RemoteApp product completion. It tightens the
  evidence gate required before such a claim can be made.

## Verification

- `tools/scripts/frontend-remoteapp-product-flow-e2e.sh --self-test`
- `tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh`
- `tests/scripts/test_check_remoteapp_frontend_product_flow_e2e.sh`

