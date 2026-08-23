# RemoteApp Multi-Window Target Identity Artifact Slice

## Intent

Close the verifier seam where RemoteApp multi-window/application tracking could
prove distinct stream/session/resource identifiers without proving that each
stream rendered the selected target's visual content.

## Boundary

This slice updates the live evidence contract and closure audit only. It does
not claim product completion, implement platform capture, or replace the real
macOS/Windows/Linux host runners required to emit the artifact.

## Product invariant

Independent RemoteApp window/application tracking is not proven by data
structure independence alone. A passing artifact must prove:

- every concurrent stream has its own selected sentinel id;
- the rendered sentinel owner equals the selected Resource URA;
- the selected sentinel rendered in that stream;
- foreign/cross-stream sentinels did not render in that stream;
- application window-set rebind renders committed window-set sentinels after
  `TARGET_REBOUND`;
- uncommitted same-app window sentinels remain absent after rebind.

## Architecture decision

Keep sentinel verification in the RemoteApp E2E artifact verifier. The daemon
and frontend can expose target IDs and stream IDs, but only the live host/browser
artifact can prove that the media plane rendered the selected target instead of
another same-app or concurrent window.

## Verification checklist

- `bash -n tools/scripts/remoteapp-multi-window-tracking-e2e.sh`
- `bash tools/scripts/remoteapp-multi-window-tracking-e2e.sh --self-test`
- negative artifact: missing `selected_sentinel_rendered` fails
- negative artifact: `foreign_sentinel_rendered=true` fails
- negative artifact: missing app rebind committed-window-set sentinels fails
- `python3 -m json.tool docs/design/remoteapp-product-readiness-matrix.json`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `git diff --check`

## Non-claims

- This does not certify live multi-window product readiness without a real
  `--run` artifact from host/browser automation.
- This does not certify input injection, media adaptation, network fallback,
  frontend lifecycle, or cross-device behavior.
