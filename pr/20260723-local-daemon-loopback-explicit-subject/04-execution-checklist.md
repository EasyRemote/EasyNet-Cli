# Execution Checklist

- [x] Identify implicit local daemon self-subject policy and callers.
- [x] Remove `LocalDaemonSelf` and `local_root` tuple constructor.
- [x] Route generic local ability helper through explicit daemon subject.
- [x] Add focused regression tests.
- [x] Add convergence gate checks.
- [x] Run focused tests, fmt, architecture gates, SPEC v2 gate, and codegraph.
