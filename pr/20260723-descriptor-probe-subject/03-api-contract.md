# API Contract

- Public C ABI/SDK behavior is unchanged.
- Error class remains `DescriptorResolutionError::InvalidRequest` for unsupported
  callee kinds.
- The internal helper name changes from target-owned descriptor subject wording
  to a closed `DescriptorCatalogProbeSubject` projection.
- The SPEC v2 gate rejects reintroducing the old helper name.
