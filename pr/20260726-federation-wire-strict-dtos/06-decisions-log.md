# Decisions Log

## 2026-07-26

- Selected federation wire DTO strictness because codegraph showed active
  product wire contracts still lacking fail-closed serde boundaries.
- Keep this separate from resolver-config strictness so each commit has one
  revert-safe architectural boundary.
- Avoid URI terminology even in negative tests. Retired-field tests use generic
  `retired_*` names so active source remains URA-only.
