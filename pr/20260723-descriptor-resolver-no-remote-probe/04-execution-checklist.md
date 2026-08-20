# Execution Checklist

- [x] Remove `RemoteDescriptorCatalogProbe`.
- [x] Remove `DescriptorCatalogProbeSubject`.
- [x] Remove remote probe invocation from `runtime_resolve_descriptor_ref_json`.
- [x] Replace remote-probe tests with fail-closed catalog-miss tests.
- [x] Update SPEC v2 gate to reject remote probe fallback.
- [x] Update legacy architecture gate to reject remote probe fallback.
- [x] Remove the now-dead remote descriptor typed submit wrapper.
- [x] Run targeted Rust tests.
- [x] Run convergence gates.
- [ ] Commit with required author.
