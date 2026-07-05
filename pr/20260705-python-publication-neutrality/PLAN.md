# Python Publication Neutrality

## Objective

Remove EasyRemote-specific public naming from the Python Publication profile while preserving daemon publication host catalogue behavior required by `docs/spec/daemon-sdk-requirements-v1.md`.

## Boundary Proof

- Ownership: publication catalogue/install projections belong to the SDK Publication profile, not an EasyRemote product facade.
- Runtime delegation: installation and listing still call host-provided target/invoke hooks and SDK ResourceRef/addressing helpers; no product-specific daemon policy is introduced.
- Read model: catalogue records remain read-model projections over daemon publication responses.
- Compatibility posture: old product-named public classes are removed rather than aliased so the SDK exposes one publication model.

## Implementation

- Rename `EasyRemotePublishedAbilityRecord` to `PublicationCatalogRecord`.
- Rename `EasyRemoteAbilityInstallProjection` to `AbilityInstallProjection`.
- Rename `EasyRemotePublicationAdapter` to `PublicationHostAdapter`.
- Rename `EasyRemotePublicationCatalogFacade` to `PublicationCatalogFacade`.
- Rename local filesystem ResourceRef helper and tests to publication host terminology.
- Update exports, tests, and SDK docs.

## Verification

- Python Publication tests.
- Python SDK test suite.
- Go SDK tests.
- SDK scaffold gate.
- Formatting, diff, and terminology scans.
