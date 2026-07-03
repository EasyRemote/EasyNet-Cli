# Step-5 deletion card — SessionDispatch JSON carrier retirement

Status: **armed, waiting for the release window** (one window from
f952c5b, per the ratified mini-RFC quadrant ruling — the window is the
rollout mechanism, not a compat layer; when it elapses the cut is
unconditional).

Owner: carrier loop. Executes as **one commit**: the codec fence in
`invoke_remote_initiator.rs` (`encode_frame`/`decode_frame`) is the
single swap point, so every site below either dies with the enum or
collapses into the fence.

## Site inventory (clean HEAD, `git grep SessionDispatch HEAD -- src/`)

| File | Sites | What dies |
|---|---|---|
| `local_session_dispatcher.rs` | 181 | device JSON read arm, v0 gun-jump reply, enum match arms, JSON fixtures |
| `daemon_invocation_service_tests.rs` | 55 | JSON-carrier test fixtures (keep the v1 quadrant tests; drop the v0 cells) |
| `bidi_dispatcher.rs` | 25 | hub v0 write arm (negotiation else-branch) |
| `invoke_remote_initiator.rs` | 21 | the `SessionDispatch` enum itself + JSON half of the codec fence + legacy builders |
| `session_escalation.rs` | 10 | escalation frames move to v1-only construction |
| `session_initiator.rs` / `boot.rs` | 6 + 6 | v0 negotiation cells (`SessionContract::legacy()` callers); `DEVICE_DISPATCH_CONTRACT_VERSION` stays at 1, the v0 acceptance path dies |
| `unary_dispatcher.rs` | 3 | forward-family JSON construction |
| `pending_dispatch.rs`, `origin_caller.rs`, `mod.rs`, `federation_invoke.rs`, `hot_agent_registrar.rs` | 1 each | re-exports / single references; `origin_caller.rs` parallel verification re-run after the cut |

Also in the same cut:
- `benches/session_frame_carrier.rs`: delete the `carrier_roundtrip`
  (JSON) and `json_encode`/`json_decode` arms — the before-side is
  archived in `docs/bench/session-frame-carrier-baseline-2026-06-12.md`.
- `presence_registry.rs`: `SessionContract::legacy()` constructor dies
  with the v0 cell (negotiation below v1 becomes a session-open error).

## Cross-repo (same window, separate commits)

- **Fed-MVP**: retire `tests/schema_compat/baselines/rust/transport/session_dispatch.json`
  only. The `carrier_v1_*.bin` golden set and the `_self__*` envelope
  baselines stay — they pin live formats.
- **Axon**: no change — proto frames are the surviving format.

## Acceptance (instrumentation already in place)

1. Filtered transport suite green on a clean worktree, both feature
   sets (`--lib` and `--features axon-pb`), citing the 397-test floor
   from 2b41796.
2. Tree-wide `git grep SessionDispatch -- src/` returns **0**.
3. Commit body cites the measured before/after (6051d45): 1 KB
   21.0 µs → 0.87 µs (24×), 64 KB 1.073 ms → 5.97 µs (180×), v1 within
   ~5% of bare InvokeRequest.

## Preconditions checked before cutting

- Workspace session's transport WIP landed (their dirty set has since
  shrunk to non-transport files; re-anchor every site by content, not
  line numbers, before cutting — step-3b (0cee062) already grew the
  bidi-side inventory past this card's counts).
- step-3b shipped both arms (43b1335 + 0cee062), so the cut also
  retires `build_remote_bidi_open_dispatch_frame` (JSON) and the
  `remote_bidi_subject_ura` helper once
  `build_remote_bidi_open_frame_for_contract` loses its v0 branch.
- Backend step 4 does not depend on the JSON shapes (it doesn't — the
  forward_invoke family is already proto; confirmed with the debt loop
  before the cut via ledger note, no letter needed).
