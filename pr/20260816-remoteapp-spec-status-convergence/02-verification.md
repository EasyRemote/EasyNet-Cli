# Verification record

## Planned checks

- `bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh`
- `bash tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh`
- Targeted Rust tests covering:
  - provider-backed route projection;
  - host-only production-route blocking;
  - pending media rebind success/failure/deadline behavior.
- `git diff --check`

## Notes

- This iteration changes SPEC status text and a static boundary gate only.
- It does not claim decoded-frame E2E or browser/backend live daemon E2E
  completion.
- EasyNet Frontend behavior is unchanged; frontend tests are not required for
  this doc/static-gate-only convergence unless the final commit grows beyond
  the current scope.

## Executed verification — 2026-08-16

- `bash tools/scripts/check-remoteapp-lifecycle-input-boundary.sh` passed.
- `bash tests/scripts/test_check_remoteapp_lifecycle_input_boundary.sh` passed
  all mutation cases.
- `cargo test -q -p easynet --features remote-desktop,headless-media configured_route_provider_projects_ice_servers_without_credentials_in_evidence --lib`
  passed.
- `cargo test -q -p easynet --features remote-desktop,headless-media pending_media_rebind --lib`
  passed 3 focused rebind tests.
- `cargo test -q -p easynet --features remote-desktop,headless-media host_only_route_keeps_production_offline_after_client_media_presents --lib`
  passed.
- EasyNet Frontend read-only targeted verification passed:
  `npm test -- --run src/lib/api/remote-desktop-protocol.test.ts src/store/media-channel-store.test.ts src/store/media-channel-invocation.test.ts src/components/easynet/DeviceMediaAccess.test.tsx`
  reported 4 files and 50 tests passed.
- `git diff --check` passed.
- `codegraph sync . && codegraph status .` reported the index already up to
  date: 1,115 files, 43,884 nodes, and 172,703 edges.
- Touched-file search for forbidden non-URA address terminology returned no
  matches.

## Superseding acceptance evidence — 2026-08-16

- Native window/application decoded-frame host acceptance, all E2E-01 through
  E2E-13 checkpoints, PERF-01 through PERF-07 structural bounds, seven static
  boundaries, and seven mutation-backed boundaries are now complete. The
  tracked authoritative host result is also recorded in
  `pr/20260816-remoteapp-configured-ice-routes/02-verification.md`.
- An authenticated EasyNet browser/backend run selected a live sentinel window
  from 28 current display/application/window Resources and completed consent,
  session creation, WebRTC negotiation, and client media acknowledgement. The
  selected Resource commits PID `50099`, native window `802`, and the exact
  sentinel title. Axon invocation `inv_b5010c7562614fbe` binds that Resource URA
  in the envelope subject, causally follows consent invocation
  `inv_1634f73d2d7b4152`, and targets the device-sponsored Remote Desktop
  SystemAgent. Session
  `rdp-cbd4ba73d8acb4573074a7ea` reached `connected` and
  report invocation `inv_5edb276608334749` projects
  `production_media_ready=true`, codec/media/client readiness true, transport
  epoch `1`, device sending, and client presenting. The separate route state
  remains explicitly host-only rather than manufacturing relay readiness.
- EasyNet's downstream dedicated-surface enum and provider registry now include
  `remote_desktop`; this fixes catalog normalization at the canonical surface
  boundary instead of introducing a generic-media fallback. Its focused
  catalog/surface/device-access suite passed 4 files / 95 tests, after which the
  live launcher exposed all 12 Remote Desktop descriptors. No product-specific
  surface or lifecycle was added to either SDK.
- Final current-branch regression passed 319/319 RemoteApp Rust tests, all
  seven static boundaries, all seven mutation-backed boundaries,
  `cargo fmt --all -- --check`, and `git diff --check`. CodeGraph is synchronized
  at 1,115 files, 43,895 nodes, and 172,765 edges; its
  `TargetTrackingEmission` trail confirms one target-state owner and one
  session event-log projection boundary.
