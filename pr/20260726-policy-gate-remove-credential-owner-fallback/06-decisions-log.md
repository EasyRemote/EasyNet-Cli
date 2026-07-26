# Decisions Log

## 2026-07-26

- Keep `local_device_owner_fact` as a bootstrap-only primitive instead of deleting it. Removing it entirely would break the first-publication trust path rather than converging ordinary policy.
- Remove ordinary policy use of local credentials because it creates a second authority path outside canonical trust/descriptor admission.
