# RemoteApp target picker host preflight verification

## Checks

- `bash tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh --self-test`
- `bash tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh`
- `bash tests/scripts/test_check_remoteapp_e2e_acceptance_boundary.sh`
- Live unavailable-daemon proof:
  `bash tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh --run --sentinel-fixture --target-kind window --out-dir <isolated-dir>`
- `git diff --check`
- `codegraph sync . && codegraph status .`
- EasyNet Frontend targeted remote desktop tests:
  `npm test -- --run src/lib/api/remote-desktop-protocol.test.ts src/store/media-channel-store.test.ts src/store/media-channel-invocation.test.ts src/components/easynet/DeviceMediaAccess.test.tsx`

## Evidence

- `bash tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh --self-test`
  - passed: `host-remoteapp-target-picker-freshness-e2e self-test ok`
- `bash tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh`
  - passed: `check-remoteapp-e2e-acceptance-boundary: ok`
- `bash tests/scripts/test_check_remoteapp_e2e_acceptance_boundary.sh`
  - passed: `test_check_remoteapp_e2e_acceptance_boundary.sh: all cases passed`
- Live unavailable-daemon proof:
  - command:
    `bash tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh --run --sentinel-fixture --target-kind window --out-dir target/e2e/host-remoteapp-target-picker-freshness/preflight-check-20260816-115007-2391`
  - expected failure observed: `rc=1`
  - failure reason:
    `daemon invocation preflight failed before launching sentinel fixture:
    daemon.pid_alive is not true; daemon.invocation_accepting is not true;
    runtime.started_at is missing`
  - report phase: `daemon_invocation_preflight`
  - report reason contains: `daemon_invocation_preflight_failed`
  - side-effect proof: `sentinel_fixture_dir_absent=true`
- `git diff --check`
  - passed
- `codegraph sync . && codegraph status .`
  - passed, index up to date
- EasyNet Frontend targeted remote desktop tests:
  - command:
    `npm test -- --run src/lib/api/remote-desktop-protocol.test.ts src/store/media-channel-store.test.ts src/store/media-channel-invocation.test.ts src/components/easynet/DeviceMediaAccess.test.tsx`
  - passed: 4 files, 50 tests
- URA-only touched-file scan:
  - command:
    `rg -n "\bURI\b|\buri\b" tools/scripts/host-remoteapp-target-picker-freshness-e2e.sh tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh tests/scripts/test_check_remoteapp_e2e_acceptance_boundary.sh pr/20260816-remoteapp-target-picker-preflight || true`
  - no matches
