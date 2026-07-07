# Java/Swift Wrappers Seam Plan

## Goal

Implement the Java and Swift P1 Convenience Wrappers projection seam required by `docs/spec/daemon-sdk-requirements-v1.md` and `sdk/conformance/cases/wrapper-profile-records.yaml`.

## Scope

- Add Java and Swift wrapper profile DTOs for file records, terminal sessions, remote desktop sessions, browser sessions, and media sessions.
- Add wrapper transport/client seams over injected transports.
- Validate owner URA, session state, profile, kind, and metadata without product HTTP/WebSocket policy.
- Exercise the seam with shared conformance fixtures in Java and Swift runtime seam tests.
- Update action-adapter reports, scaffold membership, parity/status docs, and wrapper conformance requirements.

## Non-goals

- No backend HTTP/WebSocket bridges.
- No terminal, browser, remote desktop, or media product session lifecycle manager.
- No storage policy, account policy, quota policy, or UI route ownership.
- No URI aliases or legacy input aliases.
- No SDK-owned product session execution.

## Verification

- `bash tools/scripts/check-java-sdk-seam.sh`
- `bash tools/scripts/check-swift-sdk-seam.sh`
- `bash tools/scripts/check-sdk-conformance-reports.sh`
- `TMPDIR=/tmp bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-ura-naming.sh`
- `bash tools/scripts/check-sdk-package-metadata.sh`
- `git diff --check`
- `bash tools/scripts/check-sdk-completion-audit.sh`
