# API Contract

No public runtime API changes.

The new conformance contract is:

- Case id: `python/easyremote_profile_extraction`.
- Required profile facades: `PublicationClient`, `PublicationCatalogFacade`,
  `HostBindingClient`, `MissionClient`, `AdminClient`, `GatewayLifecycleFacade`,
  `AddressingClient`, and `IdentityClient`.
- Forbidden consumer semantics: `raw_publication_carrier`,
  `raw_host_stream_codec`, `raw_mission_carrier`, `raw_admin_carrier`,
  `raw_addressing_helper`, `raw_descriptor_ref_assembly`,
  `raw_ura_shape_literal`, `raw_transport_module`, and
  `sdk_internal_runtime_transport`.

Errors are reported as `BoundaryViolation` records with rule, detail, path, and
line. The shell gate returns non-zero when any violation is present.
