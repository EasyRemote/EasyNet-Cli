Invariants
==========

- Ability authority source selection must be explicit before registry assembly.
- Device authority, realm authority, and declared agent roots remain represented
  by the `AbilityAuthoritySet` state machine, not by caller-side defaults.
- Local environment authority discovery is allowed only through the explicit
  `from_local_environment` constructor.
- Tests and future registry callers must choose fixture/device/realm/local
  authority construction intentionally.
