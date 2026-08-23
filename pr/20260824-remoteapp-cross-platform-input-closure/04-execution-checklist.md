# Execution checklist

- [x] Inventory existing consent, policy, target guard, mapping, and event paths.
- [x] Add Windows bounded SendInput backend.
- [x] Add Linux X11 bounded XTest backend and Wayland fail-closed state.
- [x] Update platform capability projection and architecture gates.
- [x] Cross-compile the Linux baseline and type-check the Windows backend up to
  two pre-existing main-crate Windows `cfg` blockers; run focused host tests.
- [x] Record live-host evidence still required before product-complete claims.
