# Java/Swift Compatibility Seam Plan

## Goal

Converge the Java and Swift P1 SDK facades with the daemon SDK SPEC by adding the Compatibility profile seam declared in `docs/spec/daemon-sdk-requirements-v1.md` and `sdk/conformance/cases/compatibility-openai-carrier-projection.yaml`.

## Scope

- Java Compatibility carrier DTOs for list-models, chat completion, streaming chat completion, file upload, file fetch, and file delete requests.
- Java Compatibility projection DTOs for model pages, chat completions, chat streams, files, and file-delete results.
- Swift Compatibility carrier/projection DTOs with the same semantics.
- Injected transport clients only; no provider transport, HTTP route, product API-key, quota, billing, SSE fanout, or backend cutover behavior.
- Conformance metadata, scaffold membership, parity docs, and fixture-backed Java/Swift tests.

## Non-Goals

- No backend HTTP/OpenAI route implementation.
- No product API-key policy, quota, billing, or multipart storage ownership.
- No daemon-owned provider transport for Java/Swift.
- No obsolete input fields beyond the v4 fixture fields declared by the Compatibility profile.
- No alternate address-family terminology.

## Verification

- `bash tools/scripts/check-java-sdk-seam.sh`
- `bash tools/scripts/check-swift-sdk-seam.sh`
- `bash tools/scripts/check-sdk-conformance-reports.sh`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-ura-naming.sh`
- `bash tools/scripts/check-sdk-package-metadata.sh`
- `git diff --check`
- `bash tools/scripts/check-sdk-completion-audit.sh`
