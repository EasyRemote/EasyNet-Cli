# Decisions and evidence

## Decision

Add `requires_media_scenarios` to the RemoteApp product-completion gate.

The aggregate gate validates only the summary fields emitted by
`remoteapp-media-adaptation-e2e.sh`:

- `scenario`
- `video_codec`
- `video_transport`
- `audio_codec`
- `selected_resource_ura`
- `media_pipeline_id`
- `render_probe_observed_at_ms`
- `measured_fps`
- `effective_fps`
- `target_bitrate_kbps`
- `observed_bitrate_kbps`
- `frames_rendered`
- `audio_packets_rendered`
- `audio_samples_rendered`
- `frames_dropped`
- `adaptation_event_types`

The deeper live artifact validation remains in the media verifier.

## Verification plan

- `bash -n tools/scripts/remoteapp-media-adaptation-e2e.sh tests/scripts/test_remoteapp_media_adaptation_e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tools/scripts/remoteapp-media-adaptation-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_media_adaptation_e2e.sh`
- `bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
- `git diff --check -- tools/scripts/remoteapp-media-adaptation-e2e.sh tests/scripts/test_remoteapp_media_adaptation_e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh pr/20260823-remoteapp-media-aggregate-scenario-gate/00-intent.md pr/20260823-remoteapp-media-aggregate-scenario-gate/01-invariants.md pr/20260823-remoteapp-media-aggregate-scenario-gate/02-decisions-and-evidence.md`

## Verification results

- `bash -n tools/scripts/remoteapp-media-adaptation-e2e.sh tests/scripts/test_remoteapp_media_adaptation_e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `git diff --check -- tools/scripts/remoteapp-media-adaptation-e2e.sh tests/scripts/test_remoteapp_media_adaptation_e2e.sh tools/scripts/remoteapp-product-completion-e2e.sh tests/scripts/test_remoteapp_product_completion_e2e.sh pr/20260823-remoteapp-media-aggregate-scenario-gate/00-intent.md pr/20260823-remoteapp-media-aggregate-scenario-gate/01-invariants.md pr/20260823-remoteapp-media-aggregate-scenario-gate/02-decisions-and-evidence.md`
- `bash tools/scripts/remoteapp-media-adaptation-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_media_adaptation_e2e.sh`
- `bash tools/scripts/remoteapp-product-completion-e2e.sh --self-test`
- `bash tests/scripts/test_remoteapp_product_completion_e2e.sh`
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`
- `bash tests/scripts/test_check_remoteapp_product_closure_audit.sh`
