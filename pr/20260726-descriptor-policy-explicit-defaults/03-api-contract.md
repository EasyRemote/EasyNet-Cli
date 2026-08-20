API contract
============

Internal Rust contract
----------------------

`Visibility` states are explicit:

- `Public`
- `Scoped`
- `Private`

`ScopeRule` states are explicit:

- `Any`
- `OnlyMatching`
- `None`

Neither enum has a default state.

Public behavior
---------------

No public wire behavior changes. Existing descriptor builders and TOML parsing
already choose these values explicitly.

Failure semantics
-----------------

Future code that tries to synthesize descriptor policy with
`Default::default()` should fail at compile time.
