# Execution checklist

- [ ] Remove Python `PrincipalProvider` alias/export.
- [ ] Remove Go duplicate `PrincipalProvider` interface and migrate `PrincipalClient`.
- [ ] Update tests to assert one canonical seam and provider-backed implementation.
- [ ] Rebuild SDK conformance inventory/parity artifacts.
- [ ] Add/adjust SPEC gate guard so `PrincipalProvider` cannot reappear.
- [ ] Run focused SDK tests and SPEC v2.
