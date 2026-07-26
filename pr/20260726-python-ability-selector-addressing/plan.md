# Python ability selector Addressing cutover

## Goal

Remove the Python SDK tuple-invocation compatibility path that classifies `ability`
strings by inspecting URA text. The SDK facade must delegate selector identity to
the canonical Addressing provider and keep the public tuple API unchanged.

## Root abstraction problem

`sdk/python/easynet_sdk/ability_invocation.py` currently decides whether a tuple
`ability` field is an Ability URA or a descriptor reference by checking
`startswith("easynet:///")`, `"/ability/"`, and absence of `@`. That duplicates
Axon grammar in the language facade and lets future grammar changes diverge from
the canonical runtime model.

## Invariants

1. Public tuple input remains compatible: callers may still pass either an
   Ability URA or a descriptor reference in the `ability` field.
2. Selector classification is provider-backed: an Ability URA is recognized only
   after `AddressingClient.project_ability_ura` accepts it.
3. Descriptor references remain validated by `AddressingClient.project_descriptor_ref`
   when the tuple is converted to a draft.
4. Runtime governance read abilities are still rejected from generic invocation
   paths and must use the dedicated receipt/catalogue providers.
5. No Python facade code may own URA path substring classification for tuple
   ability selectors.

## Boundary proof

The facade owns object-shape adaptation only. Addressing owns identity projection.
Runtime owns descriptor resolution. This refactor moves selector identity from
facade text checks into Addressing projection, so the SDK continues to expose a
simple tuple adapter without embedding product or grammar-specific parsing.

## Verification plan

- Python targeted ability invocation tests.
- Python bytecode compile for changed Python files.
- canonical runtime convergence v2 gate.
- v2 self-test if the targeted gate passes.
- `git diff --check`.

