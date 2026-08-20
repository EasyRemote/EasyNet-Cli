API contract
============

Input
-----

An invocation builder with any of:

- `caller_ura` containing `00000000-0000-0000-0000-000000000000`
- `callee_ura` containing `00000000-0000-0000-0000-000000000000`
- `subject_ura` containing `00000000-0000-0000-0000-000000000000`

Output
------

The builder returns/raises `INVALID_ARGUMENT` before producing an
`InvocationDraft`.

Preserved behavior
------------------

- Builder method names remain unchanged.
- Non-empty non-sentinel strings continue to be accepted at this boundary.
- Descriptor canonicality remains validated by addressing/runtime provider
  boundaries.
