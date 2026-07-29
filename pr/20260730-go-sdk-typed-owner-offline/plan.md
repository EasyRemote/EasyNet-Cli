# Go SDK typed descriptor owner-offline

Date: 2026-07-30

## Goal

Remove Go SDK descriptor owner-offline business classification from message
substrings. The Go facade must consume canonical error codes or structured
error details from the runtime/FFI boundary, matching the typed descriptor
resolution model.

## Invariants

1. `DESCRIPTOR_OWNER_OFFLINE` is accepted when it appears as a canonical error
   code.
2. Structured details may carry a canonical runtime code, but free-form
   messages must not be parsed for route-negative business state.
3. `ABILITY_NOT_FOUND` remains descriptor not found unless the producer supplies
   a canonical owner-offline code.
4. Direct runtime gRPC errors map owner-offline only from typed status details
   or canonical code text, not from `ROUTE_NEGATIVE` substrings.
5. Public error compatibility is preserved for canonical producers.

## Boundary decision

The runtime/FFI boundary owns descriptor resolution state. The Go SDK owns
presentation of SDK errors. It may normalize structured canonical codes, but it
must not infer provider state from lower-layer English/diagnostic text.

## Verification plan

- Update Go error tests to require canonical code/details.
- Add a negative test proving route-negative text with `ABILITY_NOT_FOUND` is
  not promoted.
- Add source gate coverage so `isDescriptorOwnerOfflineMessage` cannot return.
- Run Go SDK targeted tests, SDK public API gate, architecture gate, SPEC v2
  gate, formatting, and cargo check as appropriate.
