# Invariants

1. RuntimeAdmin route names are owned by the manifest and generated bindings.
2. Go and Python SDKs remain two implementations of the same runtime model.
3. `runtime_admin`, `session.list`, and `federation.revoke` preserve their
   exact public behavior and descriptor references.
4. Daemon `session.list` and `federation.revoke` aliases continue to expose
   the same names while delegating ownership to generated bindings.
5. Generated-file freshness must be executable in CI through `--check`.
6. No EasyNet-specific abstraction is added to the SDK API; the SDK keeps
   generic runtime administration concepts only.
7. No legacy fallback path is introduced.
