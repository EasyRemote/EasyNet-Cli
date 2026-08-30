# RemoteApp media adaptation comparability artifact contract

## Product gap

The media adaptation verifier requires baseline, degraded-network, and
backpressure scenarios, but those scenarios could be emitted from unrelated
targets or different codec/transport pipelines. In that case bitrate, FPS, and
drop deltas are not product evidence for adaptation; they are only unrelated
measurements that happen to satisfy numeric comparisons.

## Boundary decision

- The verifier validates evidence from a real media runner; it does not encode,
  capture, or simulate media.
- The runner owns producing comparable scenarios under controlled conditions.
- The artifact must bind all passed scenarios to one selected Resource URA and
  one media pipeline identity, with stable video codec/transport and audio
  codec across the comparison.

## Invariants

1. Every passed scenario must include a non-empty `media_pipeline_id`.
2. Baseline, degraded-network, and backpressure scenarios must share the same
   `selected_resource_ura`.
3. The scenarios must share the same `media_pipeline_id`.
4. The scenarios must share the same video codec and transport.
5. The scenarios must share the same audio codec.
6. Numeric deltas remain required: degraded target/observed bitrate must be
   lower than baseline, degraded FPS must drop or frames must be dropped, and
   backpressure dropped frames must exceed baseline.

## Verification checklist

- `bash -n tools/scripts/remoteapp-media-adaptation-e2e.sh` — passed
- `python3 -m json.tool docs/design/remoteapp-product-readiness-matrix.json
  >/dev/null` — passed
- `bash tools/scripts/remoteapp-media-adaptation-e2e.sh --self-test` — passed
- negative `--run --evidence-json` fixture with mismatched
  `media_pipeline_id` — failed as expected
- negative `--run --evidence-json` fixture with mismatched selected Resource
  URA — failed as expected
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh` — passed
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh` — passed
- `git diff --check` — passed
