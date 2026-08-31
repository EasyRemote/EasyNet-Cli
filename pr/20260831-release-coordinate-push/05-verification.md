# Verification

Planned proofs cover malformed versions, no-write checks, private Node/Python isolation, lock drift, exact Axon revision binding, generator failure, transaction rollback, dirty-tree rejection, protected-branch rejection, and successful metadata commit identity.

Executed evidence:

- `python3 tools/scripts/test_update_project_version.py`: PASS, 3 tests.
- `python3 tools/scripts/test_update_axon_dependency.py`: PASS, 3 tests.
- `python3 tools/scripts/test_release_coordinate.py`: PASS, 5 tests including detached check mode and a real non-force push to a local bare remote.
- Existing Axon-lock self-tests: PASS, 6 tests.
- Existing workflow-integrity self-tests and live check: PASS.
- Clean Axon worktree lock verification: PASS.
- `go test ./...` in `sdk/go` through root `go.work`: PASS.
- ShellCheck, Ruff formatting, and Ruff lint: PASS.
- `python3 tools/scripts/release-coordinate.py --check` from clean detached CLI/Axon worktrees: PASS; Tide candidate `0.150.8`, isolated Python toolchain bootstrap, Go conformance, and exact Axon lock verification all completed without changing the caller.
