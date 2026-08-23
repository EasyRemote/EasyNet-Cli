# RemoteApp view-only attach InvokeBidi full-URA gate

## Product seam

The view-only input safety harness already exercises the public
`remote_desktop.attach` InvokeBidi path by resolving the descriptor to a full
Ability URA and invoking `run_easynet ability bidi "$ATTACH_ABILITY_URA"`.
However `check-remoteapp-e2e-acceptance-boundary.sh` still required the short
name form `ability bidi remote_desktop.attach`. That contradicts the current
descriptor-bound architecture and another frontend product-flow gate that
rejects short attach ability names.

## Invariants

- The E2E harness must invoke attach through `run_easynet ability bidi
  "$ATTACH_ABILITY_URA"`, not a short name.
- The harness must still prove the resolved Ability URA addresses
  `remote_desktop.attach` and has `call_mode == "bidi"`.
- The evidence must still record `input_transport == "axon_invoke_bidi"`,
  subject binding, causal context, input client sequence, and client send
  timestamps.
- The checker self-test must fail if the Bidi invocation is skipped.

## Expected impact

This removes a checker-level architecture contradiction without weakening the
product requirement. The accepted proof is now aligned with descriptor-bound
public ability invocation and remains stricter than the old short-name probe.

## Verification

- Initial failure:
  `bash tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh` rejected the
  current harness because the checker required the obsolete short-name attach
  invocation.
- Initial frontend-flow failure:
  `bash tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh` rejected the
  audit because the visible media support contract was split across a line
  break instead of carrying the exact visible `media_pipeline_support` evidence
  phrase.
- Passed:
  `bash tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh`
- Passed:
  `bash tests/scripts/test_check_remoteapp_e2e_acceptance_boundary.sh`
- Passed:
  `bash tools/scripts/check-remoteapp-frontend-product-flow-e2e.sh`
- Passed:
  `bash tests/scripts/test_check_remoteapp_frontend_product_flow_e2e.sh`
- Passed:
  `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- Passed:
  `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
