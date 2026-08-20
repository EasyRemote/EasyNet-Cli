Implementation plan:

1. Add a daemon-side helper that recognizes route-negative owner-offline
   failures and maps them to `Unavailable`.
2. Add Go direct-runtime canonicalization for owner-offline gRPC details.
3. Add Python direct-runtime canonicalization with identical semantics.
4. Add failure-path tests in Rust, Go, and Python.
5. Update SPEC v2 gate if it still allows owner-offline to remain in
   `ABILITY_NOT_FOUND` projection.
6. Run targeted tests, fmt, architecture gate, SPEC v2 gate, and diff check.
