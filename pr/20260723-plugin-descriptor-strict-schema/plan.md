# Plugin ability descriptor strict schema convergence

## Goal

Remove the plugin ability descriptor compatibility behavior where installed
`*.ability.toml` files could carry unknown fields that the daemon silently
discarded before projecting the runtime registry manifest.

## Root abstraction problem

`plugin.toml` declares package ownership and ability rows, while each
`*.ability.toml` declares the ability descriptor facts used for runtime
publication. If descriptor parsing is permissive, a plugin author can declare
hidden route, proof, schema, or receipt facts that never reach the canonical
runtime manifest. That creates a second descriptor model at the plugin boundary.

## Invariants

1. Installed plugin ability descriptors reject unknown fields at typed parse.
2. Descriptor parse failures stay typed as `DescriptorParseFailed`.
3. Schema-specific payloads remain open only inside `input_schema` and
   `output_schema`, where JSON Schema owns interpretation.
4. Existing valid installed package descriptors continue to parse unchanged.
5. Runtime descriptor projection remains derived only from typed descriptor
   facts.

## Verification plan

- Focused package tests for unknown descriptor field rejection.
- Existing package tests to cover installed/builtin package parsing.
- `cargo fmt --check`.
- SPEC v2 convergence gate.
- Architecture convergence gate.
- codegraph sync/status.

