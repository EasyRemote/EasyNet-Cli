# Decisions and Evidence

## Decision

Use the committed daemon ability catalog as the authority for lifecycle Ability
URAs:

- `remote_desktop.end_session` resolves to one rpc Ability URA before timeout
  and cancel cleanup calls.
- `remote_desktop.refresh_lease` resolves to one rpc Ability URA before resume
  lease refresh.
- Lifecycle calls derive `--causal-context-json` from
  `session.consent.approval_receipt` returned by `create_session`.
- The lifecycle harnesses share
  `tools/scripts/remoteapp-lifecycle-harness-lib.sh` for catalog URA resolution
  and approval receipt causal-context projection.
- The resume harness resolves catalog URAs before session creation and uses
  wider default lease timing so public refresh evidence is not dependent on
  local command overhead.
- Static product-closure audit rejects direct short-name lifecycle invocation.
- Static product-closure audit rejects `--causal-root` in lifecycle harnesses.

## Why

The CLI's strict URA requirement is the correct public boundary. The harness was
wrong because it treated descriptor names as invoke targets. Adding a short-name
fallback to `ability invoke` would hide an incomplete public Invocation tuple and
make product evidence weaker.

The daemon/plugin consent check is also correct: timeout, cancel, and resume are
session-bound calls, so their causal context must prove the original consent
approval receipt instead of declaring a new root call.

## Evidence to collect

- `host-remoteapp-session-timeout-e2e.sh --self-test`
- `host-remoteapp-session-cancel-e2e.sh --self-test`
- `host-remoteapp-session-resume-e2e.sh --self-test`
- `check-remoteapp-product-closure-audit.sh`
- Live timeout/cancel/resume runs against the local daemon after the harness
  boundary fix.

## Collected evidence

- `host-remoteapp-session-timeout-e2e.sh --self-test`: pass.
- `host-remoteapp-session-cancel-e2e.sh --self-test`: pass.
- `host-remoteapp-session-resume-e2e.sh --self-test`: pass.
- `check-remoteapp-product-closure-audit.sh`: pass.
- Window target live reports:
  - `target/e2e/host-remoteapp-session-timeout/20260823-live-window-causal-222646-11519/report.md`
  - `target/e2e/host-remoteapp-session-cancel/20260823-live-window-causal-222700-12564/report.md`
  - `target/e2e/host-remoteapp-session-resume/20260823-live-window-stable-222830-19233/report.md`
- Application target live reports:
  - `target/e2e/host-remoteapp-session-timeout/20260823-live-application-causal-222846-20408/report.md`
  - `target/e2e/host-remoteapp-session-cancel/20260823-live-application-causal-222859-21255/report.md`
  - `target/e2e/host-remoteapp-session-resume/20260823-live-application-stable-222859-21261/report.md`
- Post-refactor shared-helper live smoke:
  - `target/e2e/host-remoteapp-session-resume/20260823-live-window-shared-helper-223303-36714/report.md`

This evidence is bounded to local macOS daemon lifecycle behavior. It does not
close permission revoke, browser/WebRTC rebind, crash/restart recovery,
cross-device transport, or cross-platform OS product evidence.
