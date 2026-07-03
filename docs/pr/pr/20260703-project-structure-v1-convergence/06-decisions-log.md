# Decisions Log

## 2026-07-03

- Decision: Treat `docs/spec/project-structure-v1.md` as final and do not edit
  it during this convergence pass.
- Decision: Keep the plan pack under `docs/pr/pr/` instead of introducing a
  top-level `pr/` root, because the final repository layout is the higher
  project-structure authority for this task.
- Decision: Accept historical docs and negative test fixtures that mention
  retired roots, but remove active test/code references that still point to
  retired ownership roots.
- Decision: Delete `.DS_Store` artifacts because final layout and hygiene gates
  require source roots to contain only intentional project files.
- Decision: Preserve `VERSION` and `README.pdf` as retained root release
  artifacts, and make the structure guard require them instead of treating them
  as old-root debt.
- Decision: Track root `Cargo.lock`; the final repository layout names it as a
  root file, so a local ignored lockfile is not sufficient.
- Decision: Rewrite active source comments that pointed at retired
  EasyNet-Cli `runtime::...` ownership paths to current semantic modules.
