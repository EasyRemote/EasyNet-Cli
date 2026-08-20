# Decisions Log

## 2026-07-08

- Kept desktop companion lifecycle in EasyNet-Cli daemon/plugin ownership because the SPEC defines it as user-session UX supervision, not Axon Invocation semantics.
- Preserved separate ability-plugin and desktop-companion runtime paths while sharing package discovery, install transactions, status projection, and CLI visibility.
- Treated package artifact hashing as a package-boundary invariant: companion executable artifacts must be declared under `bin/` or `dist/`, the directories included in the installable-surface hash.
- Fixed Java SDK companion list parsing at the DTO boundary rather than compensating in tests or transport fixtures.
