# Backend Boundary Self-Test Hygiene Plan

## Goal

Keep the backend SDK-only boundary self-test hermetic by storing its expected
failure output inside the per-test temporary directory.

## Scope

- Replace the shared `/tmp/backend-sdk-only-boundary-self-test.out` path with a
  path under the `mktemp` fixture directory.
- Preserve all existing forbidden-boundary assertions.
- Verify the self-test and aggregate SDK readiness gate.

## Non-Scope

- No backend scanner rule changes.
- No SDK profile status changes.
- No product cutover claim.
