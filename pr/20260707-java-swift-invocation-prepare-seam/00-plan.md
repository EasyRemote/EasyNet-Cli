# Java/Swift Invocation Prepare Seam Plan

## Goal

Converge Java and Swift Runtime Core seams with the canonical Invocation lifecycle:

`InvocationDraft -> PreparedInvocation -> SignedInvocation -> submitSigned`

This iteration covers shared seam capability for:

- `invocation/canonical_material`
- `invocation/prepared_not_submittable`
- `invocation/presigned_submit`

## Scope

- Add immutable Java and Swift DTOs for `SigningMaterial`, `InvocationSignature`, `PreparedInvocation`, `SignedInvocation`, and `InvocationHandle`.
- Add runtime transport methods for `prepare` and `submitSigned` over canonical JSON bytes.
- Keep canonical material provider-backed by the injected runtime transport; the facades do not compute canonical bytes.
- Reject direct submission of `PreparedInvocation` at the SDK boundary before transport.
- Preserve caller signature and daemon-provided canonical material without rewriting either.
- Update conformance reports and docs for the Java/Swift seam state.

## Non-Goals

- No local daemon signer implementation.
- No provider-backed C ABI or daemon transport adapter for Java/Swift.
- No handle terminal monotonicity claim in this slice.
- No product-specific lifecycle, receipt shape, directory model, or EasyRemote/EasyNet-specific abstraction.

## Verification

- `tools/scripts/check-java-sdk-seam.sh`
- `tools/scripts/check-swift-sdk-seam.sh`
- `tools/scripts/check-sdk-conformance-reports.sh`
- `tools/scripts/check-sdk-scaffold.sh`
- `tools/scripts/check-sdk-ura-naming.sh`
- `tools/scripts/check-sdk-package-metadata.sh`
- `git diff --check`
