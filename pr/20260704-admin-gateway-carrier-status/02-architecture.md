# Architecture

## Layering

- `daemon::admin_gateway_contract`: shared SDK Admin + Gateway object model for
  carrier construction and status/record projection.
- `ffi::admin_gateway`: C ABI projection. Owns pointer, handle, UTF-8, JSON
  decode, and output string allocation only.
- `sdk/schemas` and `sdk/conformance`: binding-facing DTO shapes and golden
  examples.
- `include/easynet_cli.h` and `docs/spec/ffi-abi-v4.md`: ABI contract.

## Boundary Proof

The daemon already owns:

- lifecycle status and control discovery under `daemon::boot::lifecycle` and
  `daemon::control::discovery`;
- hosted agent registry rows through `agent.list/start/stop/refresh`;
- session listing through `session.list`.

The SDK should expose these as typed Admin + Gateway DTOs and complete
Invocation carriers. Backend OAuth/JWT, pairing-token minting, certificate
policy, and browser session UX remain product/backend concerns.

## Module Shape

Use small value objects for gateway readiness classification and admin ability
selection. Avoid duplicating the shared carrier builder or parsing daemon
status strings at the FFI boundary.
