# Device ability bundle envelope convergence

## Goal

Remove the deploy-time compatibility behavior where `AbilityManifest` silently
ignored provider-specific fields from `ability.json`. Device ability deployment
must keep a clean boundary between:

- the canonical runtime ability manifest, and
- the product deploy bundle envelope that supplies the device route namespace.

## Root abstraction problem

`AbilityManifest` is the canonical runtime descriptor source, but
`ability.deploy` also used the same JSON object to carry `namespace`. Because
the manifest DTO did not deny unknown fields, deploy accepted arbitrary
provider metadata (`category`, `command`, `tool_name`, etc.) and silently
dropped it before binding the runtime descriptor. That makes source attestation
and product behavior diverge: the operator thinks those fields are part of the
installed bundle while the runtime proves only the typed subset.

## Invariants

1. `AbilityManifest` rejects unknown top-level and nested fields.
2. `ability.deploy` accepts exactly one deploy-envelope extension:
   `namespace`.
3. Runtime install hashes, durable records, restore, and descriptor binding use
   canonical manifest bytes with `namespace` removed.
4. Unknown provider metadata fails before registrar mutation.
5. Missing/invalid namespace still fails before registrar mutation.
6. Public deploy behavior remains compatible for valid bundle authors:
   `ability.json` may still include `namespace`; the runtime just no longer
   treats it as part of the canonical manifest.

## Verification plan

- Focused Rust tests for `AbilityManifest` strict parsing and device ability
  deployment envelope handling.
- `cargo fmt --check`.
- SPEC v2 convergence gate.
- Legacy architecture convergence gate.
- codegraph status after edits.

