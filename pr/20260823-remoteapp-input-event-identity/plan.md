# RemoteApp input event identity

## Intent

Make real RemoteApp input application events bindable by independent platform
observers. The current input plane records timing and client sequence, but the
live input-injection product verifier requires a stable `input_event_id` so an
observed OS pointer/key effect can be correlated to the exact daemon-applied
input frame.

## Architecture boundary

- Axon Invocation semantics do not change.
- RemoteDesktopPlugin remains the AbilityImpl and owns the high-frequency input
  data-channel execution state.
- `input_event_id` is daemon-local RemoteApp execution evidence attached to the
  `INPUT_FRAME_APPLIED` session event. It is not a new Invocation id, receipt
  id, session id, or frontend-generated authority token.

## Invariants

1. Every accepted pointer/key frame emitted as `INPUT_FRAME_APPLIED` carries one
   non-empty `input_event_id`.
2. The id is derived from session id, transport epoch, input kind/action,
   accepted counter, client sequence when present, and host receive time.
3. Rejected frames do not mint applied-input ids.
4. The id is opaque; consumers must compare it for equality, not parse policy
   from it.
5. Existing client sequence, timing, target geometry revision, and focus epoch
   projections remain unchanged.

## Implementation checklist

- Add deterministic input-event id construction in `plugins/remote-desktop/src/input.rs`.
- Include session id and transport epoch in the applied payload boundary so the
  id can bind to the actual media/input generation.
- Extend unit coverage for presence, stability, session binding, and
  same-sequence/different-session separation.
- Update readiness evidence without claiming full input-injection product
  completion.

## Verification

Commands to run before commit:

```bash
cargo test --features axon-pb --lib remote_desktop::input
bash tools/scripts/check-remoteapp-product-closure-audit.sh
git diff --check
```

## Results

- `cargo test --features axon-pb --lib remote_desktop::input` — passed, 27
  tests.
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh` — passed.
- `bash tools/scripts/remoteapp-input-injection-e2e.sh --self-test` — passed.
- `rustfmt --edition 2021 --check plugins/remote-desktop/src/input.rs` —
  passed.
- `git diff --check` — passed.

## Non-claims

This change does not claim full input-injection product completion. The live
`remoteapp-input-injection-e2e.sh` artifact is still required to prove real OS
effects, focus binding, coordinate tolerance, and terminal receipts.
