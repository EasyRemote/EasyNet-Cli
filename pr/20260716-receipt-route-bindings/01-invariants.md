Invariants

1. `provider_routes/easynet-receipt-routes.v1.json` is the only editable source
   for Receipt provider route ability names in this slice.
2. Generated Receipt constants are internal to their language package/module and
   daemon crate; no new public SDK API surface is introduced.
3. Existing Go constant identifiers, Python `_RuntimeReceiptAbility` attributes,
   and daemon `names::governance::*` identifiers remain valid aliases to
   preserve internal call sites.
4. Receipt ledger parsing, causality projection, receipt verification, trace
   graph parsing, cursor bounds, and page terminality do not change.
5. Principal and AccessControl route generators keep byte-equivalent generated
   output after moving onto the shared generator core.
6. Remaining literal Receipt route strings are allowed only in the manifest,
   generated files, user-facing diagnostic text, and tests/assertions that
   verify wire behavior.
7. Generator `--check` must fail when any generated binding is stale.

Boundary proof

- Axon owns receipt verification, causal context, and ledger/trace semantic
  projection. This slice does not redefine those structures.
- EasyNet-Cli daemon owns the receipt-history and trace abilities as daemon
  governance/read-model surfaces.
- Go/Python SDKs own provider facades that invoke daemon abilities, but they do
  not own independent route spelling.
- The shared generator core is build-time tooling; it does not enter runtime
  policy, admission, or receipt execution paths.
