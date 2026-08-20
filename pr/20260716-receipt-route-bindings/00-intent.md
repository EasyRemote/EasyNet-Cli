Intent

Converge Receipt route ability names onto the shared provider route manifest
pipeline without creating a third copy-pasted generator.

The duplicated route set is:

- `invocation.history.list`
- `invocation.history.get`
- `invocation.trace.get`

These names are currently handwritten in the Go SDK, Python SDK, and daemon
governance ability names. This slice makes a manifest the editable source of
truth, generates package-local constants for each consumer, and extracts common
provider route generator mechanics so Principal, AccessControl, and Receipt do
not fork generation logic.

Expected effect

- Architecture convergence: route spelling has one owner per capability.
- Code quality: generator validation/render/write logic is shared instead of
  copied per capability.
- Product consistency: Go/Python Receipt providers and daemon receipt-history
  abilities use the same route table.
