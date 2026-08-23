# RemoteApp input applied event coverage

## Intent

Fix an execution-path evidence gap in RemoteApp input injection. The input data
channel currently emits `INPUT_FRAME_APPLIED` only for the first accepted input
and every 120th accepted input. A live pointer+keyboard E2E can therefore apply
both OS effects while emitting an applied event for only the first kind, leaving
the second effect unbindable to daemon execution evidence.

## Architecture boundary

- RemoteDesktopPlugin remains the daemon-owned AbilityImpl.
- High-frequency pointer/key frames stay on the negotiated WebRTC data channel.
- The fix changes only bounded session event projection for accepted input
  frames; it does not create new Invocation semantics, receipts, or frontend
  authority.

## Invariants

1. The first accepted input frame for each input kind emits
   `INPUT_FRAME_APPLIED`.
2. Accepted input storms remain bounded by the existing periodic sample rule.
3. Rejected frames still use the rejection coalescer and do not mint applied
   event ids.
4. The applied event payload still carries `input_event_id`, timing,
   `client_sequence`, target geometry revision, and focus epoch.

## Verification

Commands to run before commit:

```bash
cargo test --features axon-pb --lib remote_desktop::input
bash tools/scripts/remoteapp-input-injection-e2e.sh --self-test
bash tests/scripts/test_remoteapp_input_injection_e2e.sh
bash tools/scripts/check-remoteapp-product-closure-audit.sh
rustfmt --edition 2021 --check plugins/remote-desktop/src/input.rs
git diff --check
```

## Results

- `cargo test --features axon-pb --lib remote_desktop::input` — passed, 28
  tests.
- `bash tools/scripts/remoteapp-input-injection-e2e.sh --self-test` — passed.
- `bash tests/scripts/test_remoteapp_input_injection_e2e.sh` — passed.
- `bash tools/scripts/check-remoteapp-product-closure-audit.sh` — passed.
- `rustfmt --edition 2021 --check plugins/remote-desktop/src/input.rs` —
  passed.
- `git diff --check` — passed.

## Non-claims

This does not prove live OS input injection. It makes the daemon execution path
emit the per-kind applied events required for a future live host runner to bind
both pointer and keyboard OS effects to daemon evidence.
