# RemoteApp media adaptation delta artifact contract

## Product gap

The media adaptation verifier requires baseline, degraded-network, and
backpressure scenarios, but it mostly validates each scenario in isolation.
That allows weak artifacts where `degraded_network` merely names a
`bitrate_downshift` event while target/observed bitrate and FPS stay equivalent
to the baseline scenario.

## Boundary decision

- The verifier still validates artifacts from a real media runner; it does not
  provision network impairment or synthesize codec behavior.
- Codec negotiation, host audio, queue bounds, and terminal receipts remain
  scenario-local evidence.
- Adaptation effectiveness is cross-scenario evidence: degraded/backpressure
  artifacts must differ from baseline in the expected direction.

## Invariants

1. `degraded_network.video.target_bitrate_kbps` must be lower than baseline.
2. `degraded_network.video.observed_bitrate_kbps` must be lower than baseline.
3. `degraded_network` must reduce effective FPS or record frame drops.
4. `backpressure.drop_policy.frames_dropped` must be higher than baseline.
5. These checks do not claim host audio or live impairment unless a real
   `--run` artifact is supplied.

## Verification checklist

- `bash -n tools/scripts/remoteapp-media-adaptation-e2e.sh`
- `bash tools/scripts/remoteapp-media-adaptation-e2e.sh --self-test`
- negative `--run --evidence-json` fixture without degraded target bitrate
  downshift must fail
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `git diff --check`

## Verified commands

- `bash -n tools/scripts/remoteapp-media-adaptation-e2e.sh`
- `bash tools/scripts/remoteapp-media-adaptation-e2e.sh --self-test`
- `python3 -m json.tool docs/design/remoteapp-product-readiness-matrix.json >/dev/null`
- negative `--run --evidence-json` fixture with degraded target bitrate equal
  to baseline rejected with
  `degraded_network target_bitrate_kbps must be lower than baseline`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `git diff --check`
