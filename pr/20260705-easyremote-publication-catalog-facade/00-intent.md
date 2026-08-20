# EasyRemote Publication Catalog Facade

## Goal

Move EasyRemote ability publication catalogue product logic into the Python
SDK while preserving EasyRemote's public AbilityControl API and error reasons.

## Boundary

- Axon remains the protocol/addressing source of truth.
- CLI daemon remains the authority for package validation, deployment,
  catalogue mutation, routing, and receipts.
- Python SDK owns the EasyRemote-facing publication facade and projections.
- EasyRemote keeps only public dataclass/API compatibility and taxonomy mapping.

## Implementation Slice

- Add `EasyRemotePublicationCatalogFacade` over
  `EasyRemotePublicationAdapter`.
- Centralize install/list/list-device/list-user/show catalogue behavior in SDK.
- Keep EasyRemote `AbilityControl` as a thin projection adapter.
- Preserve existing `ability_package_not_directory`, `empty_node`,
  `invalid_ability_scope`, `empty_user_id`, `missing_user_id`, and
  `ability_not_found` reasons.
- Update SDK status documentation to remove publication product extraction from
  the remaining cutover list.

## Non-Goals

- Do not change the daemon SDK requirements spec.
- Do not add legacy fallback paths or raw EasyRemote catalogue filters.
- Do not implement Pipeline live-tail/conformance, full AgentControl/Server
  cutover, or Axon-backed cryptographic receipt verification in this slice.
