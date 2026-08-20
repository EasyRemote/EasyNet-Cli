# Decisions Log

- 2026-07-07: Treat installed desktop companion artifacts as version-owned platform state keyed by package id and package version.
- 2026-07-07: Remove legacy shared app install target semantics; install/update now replace the version-owned target path.
- 2026-07-07: Treat a launcher entry pointing at another version as not owned by the current remove/disable operation.
