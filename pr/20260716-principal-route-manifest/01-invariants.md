Invariants

1. Principal public DTOs, clients and provider method names remain source
   compatible.
2. EasyNet ability literals are provider route facts, not canonical SDK
   domain facts.
3. Go, Python, and Rust CLI principal providers consume generated route
   constants from the same manifest.
4. The generated files carry a manifest SHA so tests can detect hand-edited or
   stale route bindings.
5. No daemon ability registration, receipt semantics, signing policy, URA
   grammar, or public conformance capability state changes in this slice.
6. Generated route constants remain package-private/internal.
7. The provider route manifest lives outside `sdk/`; SDK packages may consume
   generated internal constants but do not own EasyNet route facts.
