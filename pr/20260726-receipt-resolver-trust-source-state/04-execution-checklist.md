# Execution Checklist

- [x] Replace optional realm trust resolver with an explicit trust-source enum.
- [x] Preserve malformed trust-anchor load errors in resolver output.
- [x] Add regression coverage for malformed trust anchor preservation.
- [x] Extend SPEC v2 gate to reject `.ok().filter(...)` fallback collapse.
- [x] Run targeted tests, fmt, gates, and codegraph.
- [x] Commit with required author if stable.
