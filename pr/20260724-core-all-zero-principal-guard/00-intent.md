# Intent

## Goal

Move all-zero principal placeholder detection into a single core identity guard so runtime, CLI, FFI, and admission code do not maintain parallel sentinel constants or subtly divergent string checks.

## Non-goals

- Do not weaken any existing rejection.
- Do not change public error variants or public wire payloads.
- Do not add compatibility for all-zero identities.

## Acceptance criteria

- Rust production code has one canonical all-zero principal sentinel owner.
- Auth session, credentials, authority metadata, and public FFI invocation tuple validation use the shared guard.
- Existing all-zero negative tests continue to pass.
- SPEC v2 gate prevents reintroducing duplicate Rust sentinel constants.
