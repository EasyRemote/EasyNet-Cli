# Remoteapp Watch Inventory Availability Verification

## Required checks

- Unit coverage for unavailable watch observations not emitting removals.
- Unit coverage that discovery availability changes are observable even with no resources.
- Regression coverage that real removals after successful scans still emit removed resource URAs.
- Static boundary coverage to keep watch availability semantics explicit.
- Existing remoteapp target, E2E, lifecycle, performance, and frontend invocation gates continue to pass.

## Commands

```sh
cargo fmt --all
cargo test -q -p easynet --features remote-desktop,headless-media watch_remote_targets --lib
cargo test -q -p easynet --features remote-desktop,headless-media remote_desktop --lib
bash tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh
bash tools/scripts/check-remoteapp-performance-boundary.sh
bash tools/scripts/check-remoteapp-target-binding-boundary.sh
bash tools/scripts/check-remoteapp-frontend-invocation-boundary.sh
npm test -- --run src/lib/api/remote-desktop-protocol.test.ts src/store/media-channel-store.test.ts src/store/media-channel-invocation.test.ts src/components/easynet/DeviceMediaAccess.test.tsx
```
