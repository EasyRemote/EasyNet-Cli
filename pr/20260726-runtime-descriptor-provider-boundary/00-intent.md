Goal: move descriptor reference resolution business logic out of the FFI invocation ABI bridge and into daemon-owned runtime descriptor provider code.

Non-goals:
- Do not change public C ABI function names or JSON DTO shape.
- Do not add product-specific SDK concepts.
- Do not change descriptor reference canonical grammar.

Acceptance criteria:
- FFI parses request bytes, delegates to daemon runtime descriptor provider, and maps typed errors to ABI errors.
- FFI no longer builds system registries, materializes descriptor catalog rows, or selects descriptor rows locally.
- Existing descriptor resolution behavior remains covered by focused tests and SPEC gates.
