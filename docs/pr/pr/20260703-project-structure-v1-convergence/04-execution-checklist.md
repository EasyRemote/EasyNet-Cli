# Execution Checklist

- [x] Read `docs/spec/project-structure-v1.md` as the normative target.
- [x] Read EasyNet runtime boundary and engineering contract constraints.
- [x] Verify final project-structure guard behavior.
- [x] Verify library and all-target compilation from the final module tree.
- [x] Verify shell boundary script test suite.
- [x] Add this docs-owned plan pack without creating a top-level `pr/` root.
- [x] Remove active old-root references that are not intentional negative test
      fixtures or historical docs.
- [x] Preserve `VERSION` and `README.pdf` as retained root release artifacts.
- [x] Extend the structure guard to require retained root artifacts and reject
      extra tracked root product files.
- [x] Re-run formatting, compile, structure, and script checks after cleanup.
- [x] Split commits by semantic boundary if the verified state is coherent.
