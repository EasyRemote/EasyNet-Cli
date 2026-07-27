Goal: remove the implicit ability selector default from `namespace.proxy_resolve`.

Problem:
- `namespace.proxy_resolve` is a backend-to-daemon public ingress surface.
- Its request currently accepts missing `ability_name` and silently projects that as an empty selector.
- That keeps a query-only compatibility shape alive at the exact route boundary, even though public ingress should expose every route-selection fact explicitly.

Non-goals:
- Do not change `namespace.resolve` canonical peer semantics.
- Do not add another backend route family.
- Do not preserve a missing-field compatibility path.

Acceptance criteria:
- `ability_name` is required at `namespace.proxy_resolve` ingress.
- Directory/listing queries can explicitly declare no ability selector with `ability_name: null`.
- Ability route queries keep using an explicit string selector.
- Peer fanout arguments preserve the explicit selector state.
- SPEC v2 gate rejects the old default/missing-field shape.
