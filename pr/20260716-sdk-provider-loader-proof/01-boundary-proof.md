# Boundary Proof

## Invariants

1. SDK production loaders do not infer provider libraries from repository
   build directories.
2. Explicit `library_path` arguments remain allowed because they are caller
   supplied deployment policy.
3. Smoke and E2E scripts may continue to pass `target/debug` artifacts
   explicitly; those paths are tooling, not SDK runtime discovery.

## Current Evidence

CodeGraph and source inspection show:

- Python `CLILibrary.load` accepts an explicit path, otherwise uses
  `ctypes.util.find_library("easynet_cli")` plus platform library names only.
- Go `cabiLibraryCandidates` accepts an explicit path, otherwise uses the
  platform library name only.
- Existing gates did not encode this invariant, so the stale development-path
  lookup could return without failing CI.

## Decision

Put the rule in `check-sdk-product-neutrality.sh`, not the daemon architecture
gate. The defect is an SDK provider-boundary leak: the SDK must not make
repository layout part of canonical provider loading.
