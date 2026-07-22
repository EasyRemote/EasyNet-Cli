# Decisions Log

## 2026-07-23

- Decision: remove transport-owned callee-as-subject loopback policy instead of
  preserving it as a compatibility branch.
- Reason: public/product ingress must never rely on missing tuple facts being
  completed by transport. Daemon-system calls may still choose daemon subject,
  but that choice belongs to the issuer before transport entry.
- Decision: keep the public `invoke_local_ability(ability, args)` signature and
  migrate its internals to explicit daemon subject tuple construction.
- Reason: this preserves CLI command behavior while removing the hidden
  transport-level fallback state.
- Decision: rename the private daemon identity helper from callee-oriented
  terminology to `local_daemon_identity_ura`.
- Reason: the same daemon identity can be used as callee and subject in
  daemon-local loopback calls; the helper name must not encode one tuple role.
