API contract
============

Internal Rust contract
----------------------

`ReceiptSemantics` states are explicit:

- `Operational`
- `StateTransition(StateTransition)`

The enum has no default state.

Public behavior
---------------

No public behavior changes. Existing descriptors still receive operational
semantics where their constructor explicitly sets it.

Failure semantics
-----------------

Future code that tries to synthesize receipt semantics with
`Default::default()` should fail at compile time.
