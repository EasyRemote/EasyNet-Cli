# RemoteApp frontend rendered-session artifact contract

## Product gap

The Browser/Tauri lifecycle verifier checks the high-level frontend sequence,
but the `webrtc_attached` and `media_presented` steps can be satisfied by weak
evidence: a passed step and a boolean frame flag. That is not enough to support
the product requirement that the frontend actually displays and controls, or
policy-blocks control of, a live RemoteApp session.

## Boundary decision

- The frontend verifier validates a real UI artifact; it does not simulate
  browser actions and does not replace daemon/host media, input, network, or
  capture E2Es.
- The verifier may require UI-observable WebRTC/media/input facts produced by a
  real runner.
- Codec, NAT, OS capture, and OS input effects remain owned by their dedicated
  E2E verifiers.

## Invariants

1. `webrtc_attached` must prove connected WebRTC state and attached media stream.
2. `media_presented` must prove a visible media element and positive rendered
   frame count.
3. `input_control_attempted_or_policy_blocked` must expose a visible UI status.
4. `policy_blocked` input must include a concrete blocked reason.
5. `input_applied` input must include positive client sequence and bounded
   latency evidence.
6. The lifecycle still ends with `end_session` and visible terminal receipt.

## Verification checklist

- `bash -n tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh`
- `bash tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh --self-test`
- negative `--run --evidence-json` fixture without rendered-frame count must
  fail
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `git diff --check`

## Verified commands

- `bash -n tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh`
- `bash tools/scripts/frontend-remoteapp-browser-lifecycle-e2e.sh --self-test`
- `python3 -m json.tool docs/design/remoteapp-product-readiness-matrix.json >/dev/null`
- negative `--run --evidence-json` fixture without `frames_presented` rejected
  with `media_presented.frames_presented must be positive`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `git diff --check`
