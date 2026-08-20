# Decisions Log

## 2026-07-25

- Decision: introduce `distribution_facade` rather than forcing historical
  package roots into `public_facade`.
- Reason: package names like `easynet_sdk` are public distribution facts. They
  should not be treated as provider-neutral source roots, but the architecture
  also should not describe them as EasyNet providers.
- Decision: strengthen provider owner validation to reject `product`, `branded`,
  and `daemon` owner labels in addition to explicit product names.
- Reason: the canonical provider registry should only admit generic runtime
  ownership language.
