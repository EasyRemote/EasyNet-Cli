# Verification

## Baseline — 2026-08-30

- Branch: `codex/ffi-descriptor-authority-tuple` at `a9f7317c8`.
- EasyNet-Cli, EasyNet-Axon, and EasyNet contain concurrent uncommitted changes;
  explicit-path safety is required.
- The readiness matrix status is `incomplete`; every named product requirement
  is `partial`.
- The existing final-product-closure pack records no accepted same-campaign
  completion evidence.
- ABI v9 unit, export, packaging, and dynamic feature-discovery checks pass, but
  this proves only the generic native stream carrier, not RemoteApp product
  readiness.

## Required proof classes

1. Focused deterministic Rust/frontend/backend tests for implementation logic.
2. Mutation-resistant static gates for forbidden fallback and evidence truth.
3. Real host capture/input artifacts for each claimed platform and target kind.
4. Real selected ICE-pair/media/terminal artifacts for each claimed route.
5. Same-campaign cross-device Browser product flow and signed final decision.

Exact commands and results will be appended as they execute.

## Continuation red-team baseline — 2026-08-30

- Frontend camera/store seam reproduced at 90/91 and now passes 92/92 after
  aligning raw frames with `blob:` and proving object-URL ownership.
- P0: Windows discovery did not publish the process-instance identity already
  required by observer/media-host; Linux observer/media-host retained xcap PID
  fallback authority; capture-eligibility rules diverged by platform/layer.
- P1: deferred create, ambiguous end, one-shot event watch, permission-pending
  lease cancellation, and inventory omission lacked requested regressions.
- CodeGraph indexed the active Frontend paths and identified `rdCreate`,
  `rdEnd`, and `startRemoteDesktopEventWatch` as the lifecycle ownership seam.
  Its full Rust index watchdog terminated at 28%; Rust call paths were therefore
  completed with repository symbol search and direct source inspection.

## ABI v9 implementation evidence — 2026-08-30

- `bash tools/scripts/check-ffi-abi-v9-header.sh`: pass.
- `bash tests/scripts/test_check_ffi_abi_v9_header.sh`: pass, including mutation
  cases for stream quotas and callback-quiescence coverage.
- `cargo test -q --features axon-pb -p easynet v9_ --lib`: 6/6 pass.
- `stream_close_waits_for_inflight_callback_and_suppresses_late_eof`: pass.
- `stream_registry_enforces_per_handle_limit`: pass.
- `bash tools/scripts/check-release-package-contract.sh`: pass.
- `bash tests/scripts/test_check_release_package_contract.sh`: pass.
- `bash tests/scripts/test_macos_sign_runtime.sh`: pass with a mock signer.
- Feature discovery accepts a legacy pre-v9 document and rejects an incomplete
  advertised v9 capability; v9 is additive rather than required from v7/v8
  runtimes.
- A locally constrained macOS dylib exported exactly the 60-symbol v9 allowlist
  and passed the dynamic v9 gate. This is implementation evidence, not proof of
  a notarized production artifact.

## Evidence still required

- Live Go/Python bindings against the compiled v9 dylib. Focused fake-C-ABI and
  ownership tests pass: Go normal/C-ABI/race suites pass; Python 154 focused
  tests pass. Rust's six new owned-stream tests pass and do not use C leases.
- Linux shared-object and Windows DLL exact-export checks on their native release
  runners. PowerShell and ELF export behavior have not been proven on this host.
- A real Developer ID signed and notarized macOS archive. The local signing test
  verifies the script contract only.
- Same-campaign RemoteApp product evidence remains absent: the local daemon is
  unpaired, the Hub endpoint returns `401`, Screen Recording permission is not
  granted, and no fresh Windows or cross-device media/input run exists.

## SDK binding evidence — 2026-08-30

- Go explicit APIs: `InvokeLeasedStream`, `OpenSignedLeasedStream`,
  `LeasedStreamHandle`, `LeasedStreamEvent`, and `LeasedPayload`. Focused normal,
  C-ABI, and race tests pass; `go vet -tags runtime_cabi ./...` passes.
- Python explicit APIs: `invoke_leased_stream`, `open_signed_leased_stream`,
  `LeasedStreamHandle`, `LeasedStreamEvent`, and `LeasedPayload`. The focused
  C-ABI/stream/runtime/runtime-ability suite passes 154/154.
- Both bindings require complete v9 discovery and all three symbols. Existing
  v8 owned-event APIs remain unchanged and explicit v9 requests do not fall
  back to v8.
- Rust `RuntimeClient::submit_stream_signed` returns typed `StreamEvent` values
  with owned `Vec<u8>` payloads. It verifies sequence, invocation identity,
  receipt chain, and submitted tuple binding without exposing tonic/protobuf.
- `cargo fmt -- --check`, `git diff --check`, SDK scaffold gates, v9 ABI gates,
  and release-package contract gates pass.
- Canonical public-API and parity artifacts were regenerated after replacing
  the migration-specific public Addressing reason with
  `ABILITY_OWNER_NOT_PUBLISHER` and classifying `AbilityOutput`,
  `make_typed_ability`, and `LeasedPayload` under their canonical owners.
  `check-sdk-canonical-public-api.sh`, its mutation self-test, and the parity
  matrix self-test all pass.

## Full SDK execution — 2026-08-30

- `go test -tags runtime_cabi ./...`: pass.
- `uv run --project sdk/python pytest -q sdk/python/tests`: 670/670 pass.
- The actual debug `libeasynet_cli.dylib` dynamically loads through the Python
  SDK, reports base ABI 7 plus the complete additive v9 extension, and enables
  `stream_v9_available=true`.
- The aggregate cutover gate reached and passed the Python and Go live-daemon
  smokes, including unary invocation, typed failure, runtime events, and a
  receipt-backed terminal stream frame. These existing smokes use the owned
  stream surface; they do not yet constitute a live v9 leased-payload transfer.
- The aggregate gate remains red for stale committed conformance reports,
  missing all-language live parity results, unfiltered development-dylib
  exports, Node package-version drift, stale sibling Axon lifecycle evidence,
  absent EasyRemote checkout, downstream backend principal drift, and a live
  PrincipalLifecycle output-content-type mismatch. These are retained as
  explicit campaign blockers rather than attributed to v9.

## Live ABI v9 leased-payload execution — 2026-08-30

- `bash tools/scripts/go-sdk-live-smoke.sh`: pass against the actual dylib and
  a hermetic daemon. The Go facade opened
  `resource.watch_remote_targets(max_events=1)` through the media SystemAgent,
  received an `application/json` data frame through ABI v9, copied and released
  the native lease, then received a terminal frame with a terminal receipt.
- `bash tools/scripts/python-sdk-live-smoke.sh`: pass through the equivalent
  Python `RuntimeAbilityClient` and `LocalRuntimeAuthorityProvider` path.
- The first fail-closed attempts rejected a bare User subject and then a
  descriptor-bound User Resource without authority. The passing path uses the
  canonical descriptor-bound subject projection plus typed delegation signed
  by the daemon-custodied paired-User key. This directly covers the earlier
  `prepared subject identity` / `AUTHORITY_REQUIRED` integration seam.
- `ManagedSigningClient.ActiveSignerForSubject` now gives Go the same bounded,
  deterministic active-key selection rule already used by Python. Its unit
  test covers rotation overlap and the typed no-signer failure.
- Full Go runtime-C-ABI tests, all 670 Python tests, Ruff, canonical public API,
  and the generated parity matrix pass after this addition.
- This is real generic v9 execution evidence. It does not change the RemoteApp
  media-plane decision: sustained H.264/Opus remains on WebRTC plus the
  plugin-private shared-memory lane.

## Targeted-session convergence audit — 2026-08-30

- CodeGraph sync completed for EasyNet-Cli (1,184 files, 49,317 symbols,
  199,497 edges) and EasyNet (530 changed files indexed). The graph placed
  daemon lifecycle ownership in the RemoteDesktop session aggregate and browser
  operation ordering in `RemoteDesktopSessionCoordinator`.
- Authoritative platform worktree:
  `codex/remoteapp-platform-convergence` at `7a59c2926`, clean.
- `bash tools/scripts/check-remoteapp-native-platform-boundary.sh`: pass.
- `bash tools/scripts/test-remoteapp-native-platform-boundary.sh`: pass.
- `bash tools/scripts/check-remoteapp-linux-x11-identity-boundary.sh`: pass.
- `bash tools/scripts/check-remoteapp-surface-eligibility.sh`: pass.
- `cargo test -p easynet-remoteapp-native-platform -p easynet-remoteapp-native-protocol -p easynet-remoteapp-native-host -p easynet-remoteapp-media-host`:
  pass (4 native-platform, 44 native-protocol, 22 media-host, 1 shared-lane
  benchmark; host/doc targets also pass).
- `cargo check --locked -p easynet --features remote-desktop,native-media`:
  pass with pre-existing dead-code warnings.
- Native-host and media-host headless feature checks: pass.
- Scoped `rustfmt --check` over every added/modified Rust file in
  `1894acdd1..7a59c2926`: pass.
- Workspace-wide `cargo fmt --all -- --check`: fail only on pre-existing
  formatting in `transport/ice_candidate_exposure.rs` and `transport/webrtc.rs`,
  neither changed by `7a59c2926`; no out-of-scope formatting was applied.
- Platform-branch SPEC gates for contract, E2E acceptance, performance, picker
  subject, platform input, session subject, and target binding: pass.
- Platform-branch full regression is not yet green:
  - lifecycle/native-host boundaries reject the removal of complete Linux
    process-owned application membership/stacking comparison from the media
    process;
  - `check-remoteapp-main-crate-implementation-tests.sh` fails
    `process_scoped_application_observer_tracks_window_set_without_display_identity`
    because its Windows fixture has no canonical `process_instance_id`.
- The current shared dirty CLI worktree passes the updated input-consent gate,
  lifecycle/input gate, native-host/performance/picker/platform-input/session-
  subject/target-binding gates, and all 30 main-crate filters. Those dirty files
  were not staged, committed, cleaned, or copied over `7a59c2926`.

## EasyNet frontend convergence audit — 2026-08-30

- Commit `666244ea3cc815b78e27d4d24343d411687df6b0` is present with the required
  author and committer and contains exactly seven frontend files.
- Focused Vitest: 3 suites, 99/99 pass. The only output is Node's experimental
  localStorage warning.
- `tsc --noEmit`: pass.
- ESLint over the seven committed files: pass.
- Frontend invocation boundary and mutation self-test: pass.
- Product closure checker, its complete mutation suite, and product
  finalization self-test: pass.

## Live product completion audit — 2026-08-30

- `packaging/release/dev-check-local-runtime.sh --json --no-fail` reports the
  installed `easynet 0.148.60` host unpaired, Runtime stopped, session not
  admitted, and Hub credential verification rejected with HTTP 401.
- A real `remoteapp-product-completion-e2e.sh --check` candidate was written to
  `/private/tmp/easynet-remoteapp-final-check.yQYt7O/report.json`.
- The candidate is `finalization_state=not_eligible`,
  `product_complete_claim=false`, and missing the signed campaign plus all 19
  required live reports. This is authoritative non-completion evidence, not a
  source failure.
