# RemoteApp media/input event target binding slice

## Intent

Close one product-evidence seam in the RemoteApp interactive desktop path:
media pipeline stats and direct input events must be independently attributable
to the selected session target. Multi-window/application capture, media
adaptation, and input-injection verification cannot rely on an external
`show_session` snapshot to prove which Resource/window/application an event
describes.

## Invariants

1. Session aggregate remains the only owner of the live target binding.
2. Event projections may carry target binding evidence, but must not mutate or
   derive alternate target truth.
3. Media stats events must bind `subject_ura`, binding epoch, target identity
   epoch, geometry revision, media source epoch, consent epoch, and transport
   epoch.
4. Input channel and runtime input-permission events must carry the same
   binding context because they prove host-side control of the selected target.
5. Existing public event names and state-machine behavior remain compatible.

## Implementation plan

- Add one shared target-context projection helper in
  `plugins/remote-desktop/src/session_events.rs`.
- Route `MEDIA_PIPELINE_STATS`, `INPUT_FRAME_*`, input channel lifecycle
  diagnostics, and input permission block/restore projections through that
  helper.
- Add focused tests proving top-level event log fields and payload fields bind
  to the current target and transport epoch.
- Extend `check-remoteapp-product-closure-audit.sh` so future edits cannot
  regress target-bound media/input evidence.

## Verification

- `rustfmt --edition 2021 --check plugins/remote-desktop/src/session_events.rs plugins/remote-desktop/src/session.rs`
- Focused RemoteApp session/session_events tests.
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh`

