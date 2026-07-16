# Decisions Log

## 2026-07-16

- Chose the repository-owned refresh tool rather than manual JSON edits.
- Scoped the slice to derived report metadata because the source refactor is
  already committed and verified.
- Kept live result output under `target/` so generated run artifacts do not
  become repository state.
