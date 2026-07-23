# Execution checklist

- [x] Extract Claude plugin-dir argument assembly into a focused helper for testability.
- [x] Cut the helper over to `.claude/skills/` only.
- [x] Add a regression test that legacy `<cwd>/skills/` does not produce `--plugin-dir`.
- [x] Add/update architecture gate coverage.
- [x] Run targeted driver tests.
- [x] Run convergence gates, format, and whitespace checks.
