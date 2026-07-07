# Java/Swift Admin + Gateway Seam Plan

## Goal

Implement Java and Swift P1 Admin + Gateway facade seams required by `docs/spec/daemon-sdk-requirements-v1.md` and `sdk/conformance/cases/admin-gateway-carrier-status.yaml`.

## Scope

- Add Java and Swift Admin + Gateway request DTOs, projection DTOs, transport interfaces, and clients over injected transports.
- Build complete Invocation carriers for agent list/start/stop/refresh and session list using shared fixtures.
- Project gateway status, agent records, lifecycle results, pairing preflight/token, device credentials, device sessions, and device admin results from daemon-owned payloads.
- Exercise pairing and device-session CRUD request carriers over injected transports.
- Update Java/Swift seam tests, action-adapter reports, scaffold membership, status docs, and Admin conformance requirements.

## Non-goals

- No daemon provider, JNI, C ABI provider, or lifecycle starter for Java/Swift.
- No certificate provisioning, ACME policy, self-signed policy, onboarding guidance, browser session model, backend account table model, or EasyRemote-specific Server facade.
- No legacy input aliases or URI terminology.
- No SDK-owned readiness derivation beyond preserving daemon-provided gateway status facts.

## Verification

- `bash tools/scripts/check-java-sdk-seam.sh`
- `bash tools/scripts/check-swift-sdk-seam.sh`
- `bash tools/scripts/check-sdk-conformance-reports.sh`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-ura-naming.sh`
- `bash tools/scripts/check-sdk-package-metadata.sh`
- `git diff --check`
- `bash tools/scripts/check-sdk-completion-audit.sh`
