# Decisions and Evidence

## Decisions

1. Fix the host view-only input E2E harness to resolve the public RemoteApp
   attach Ability URA before invoking the CLI path that expects
   `<ability-ura>`. This keeps the CLI strict and makes the harness match the
   governed descriptor surface.
2. Chain the diagnostic `remote_desktop.attach` Bidi invocation to the
   `create_session.session.consent.approval_receipt` through scalar
   `--causal-context-json`. Attach is not a root call once the session has an
   approval receipt.
3. Preserve view-only input rejection semantics in the RemoteApp input policy
   layer. A view-only session must reject key/pointer frames as
   `input_scope_unsupported` even when the diagnostic preview has not delivered
   its first media frame yet. Non-view-only/display-global target loss still
   reports `target_input_not_ready`.

## Required evidence

- `host-remoteapp-view-only-input-safety-e2e.sh --self-test`
- live product-flow reaches past the previous `invalid Ability URA
  "remote_desktop.attach"` failure
- live product-flow reaches past the previous `consent_receipt_mismatch`
  failure by using the approval receipt causal context
- live product-flow preserves pointer/key `input_scope_unsupported` telemetry
  for view-only window sessions
- product-flow checker/mutation tests stay green
- product closure audit records the new live evidence scope

## Evidence captured on 2026-08-23

- `bash tools/scripts/host-remoteapp-view-only-input-safety-e2e.sh --self-test`
  passed.
- `bash tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh` passed.
- `bash tests/scripts/test_check_remoteapp_frontend_product_flow_e2e.sh`
  passed, including mutations for short attach ability names and root attach
  causal placement.
- `cargo test --lib remote_desktop::invoke_bidi -- --nocapture` passed after
  adding coverage for both view-only target loss and display-global target loss.
- `cargo test --lib remote_desktop::input -- --nocapture` passed.
- `cargo build --bin easynet` and `cargo build --bin easynet-daemon` passed.
- Live product-flow passed after rebuilding and restarting the daemon binary:
  `target/e2e/frontend-remoteapp-product-flow/20260823-daemon-binary-policy-44867/report.md`.

## Follow-up constraint

Live RemoteApp E2E depends on the `easynet-daemon` binary, not just the CLI
binary. Product-flow verification after daemon/plugin changes must rebuild
`cargo build --bin easynet-daemon` and restart the daemon before trusting live
results.
