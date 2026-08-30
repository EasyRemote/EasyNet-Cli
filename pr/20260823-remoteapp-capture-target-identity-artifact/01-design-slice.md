# RemoteApp capture target-identity artifact contract

## Product gap

The cross-platform capture verifier requires rendered frames and exact target
binding, but a positive `frames_rendered` count alone does not prove the frame
belongs to the selected window/application. A runner could render any frame and
still satisfy the current artifact contract.

## Boundary decision

- The verifier validates evidence from a real host capture runner; it does not
  implement platform capture or synthesize host windows.
- The runner owns sentinel setup and image comparison.
- The verifier requires the artifact to state that selected sentinel content was
  rendered, and for window/application sessions that unrelated sentinel content
  was absent.

## Invariants

1. Every passed capture scenario must include `selected_sentinel_rendered=true`.
2. Passed window/application scenarios must include
   `unrelated_sentinel_rendered=false`.
3. Unsupported Windows/Linux scenarios remain explicit product unsupported
   states and must not create sessions, render frames, or start display
   fallback.
4. These checks strengthen target-identity evidence without claiming unsupported
   platform backends exist.

## Verification checklist

- `bash -n tools/scripts/remoteapp-cross-platform-capture-e2e.sh` — passed
- `python3 -m json.tool docs/design/remoteapp-product-readiness-matrix.json
  >/dev/null` — passed
- `bash tools/scripts/remoteapp-cross-platform-capture-e2e.sh --self-test` —
  passed
- negative `--run --evidence-json` fixture without selected sentinel evidence
  — failed as expected with `selected_sentinel_rendered must be true`
- negative `--run --evidence-json` fixture with unrelated sentinel leakage —
  failed as expected with `unrelated_sentinel_rendered must be false`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh` — passed
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh` — passed
- `git diff --check` — passed
