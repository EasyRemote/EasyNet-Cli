# Purge Publication Mode Owner Gate

## Objective

Close the A63 regression surface with an executable architecture rule. Device-capable daemon modes must have one explicit purge publication recovery owner before transport boot can recover lifecycle state or expose device-owned mutations.

## Invariants

1. `InvocationModeCapabilities` is the sole boot-time owner for daemon mode capability classification.
2. Device mode owns purge publication recovery through the upstream `session.open` channel and its session-ready outbox hook.
3. Hub mode does not own a device purge publication outbox.
4. Both mode remains fail-closed until it has a real publication/session recovery owner; it must not inherit Device or Hub behavior implicitly.
5. Transport boot validates mode capabilities before purge lifecycle recovery.
6. A session-ready hook must redrive durable purge publication outbox work when the upstream session owner becomes available.

## Effect

This slice does not change public behavior. It converts the existing A63 runtime design into a CI-enforced contract so future edits cannot silently reintroduce a stranded committed purge outbox in `DaemonMode::Both`.
