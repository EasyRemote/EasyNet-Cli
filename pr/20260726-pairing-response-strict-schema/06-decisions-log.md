# Decisions log

## 2026-07-26

- Treat unknown pairing validate response fields as product API version skew,
  not forward-compatible extension space. This prevents retired aliases from
  riding through validate response into credentials projection.
- Remove `Default` from pairing response DTOs with required lifecycle facts so
  tests cannot normalize incomplete product credentials as ordinary values.
- Keep optional product metadata fields as serde defaults for older Hub
  responses only where they are not credential-completeness facts; unknown field
  carriers are still rejected.
