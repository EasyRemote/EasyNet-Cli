Layering:
- `src/daemon/resources/pages/pages_listener.rs`: HTTP transport boundary.
- `src/daemon/resources/pages/pages_http_projection.rs`: HTTP byte projection adapter.
- `src/daemon/ability/builtins/resources/pages/fetch.rs`: provider-backed Pages fetch ability handler.

Refactor:
- Rename `pages_serve_ability` to `pages_http_projection`.
- Keep direct local fetch consumption explicit and bounded.
- Gate old pseudo-ability vocabulary instead of leaving it as future migration prose.
