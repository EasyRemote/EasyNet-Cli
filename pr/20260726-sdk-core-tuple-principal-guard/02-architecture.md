Architecture
============

Root abstraction
----------------

`InvocationDraft` is the SDK's immutable complete runtime tuple. A tuple with an
all-zero principal is not a valid runtime tuple, regardless of provider or
product.

Boundary
--------

- Go: `sdk/go/invocation.go::InvocationBuilder.inspectDraft`.
- Python: `sdk/python/easynet_sdk/invocation.py::InvocationBuilder._inspect_draft`.

The guard is intentionally shared with existing identity guard helpers instead
of duplicated string matching.

Language parity
---------------

Java already rejects all-zero caller/callee/subject in `InvocationTuple`.
Swift already rejects all-zero tuple fields. This task converges Go/Python to
the same capability state.
