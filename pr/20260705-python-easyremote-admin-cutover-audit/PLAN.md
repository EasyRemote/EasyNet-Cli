# Python EasyRemote Admin Cutover Audit Plan

## Goal

Extend EasyRemote cutover auditing so product code cannot regress to raw daemon Admin + Gateway system ability literals now that the SDK profile bridge owns those dispatch/projection paths.

## Boundary Proof

- The audit runs against EasyRemote-like consumer source trees, not SDK internals.
- String-literal checks stay narrow to daemon system ability names owned by SDK Admin/Mission/Publications facades.
- Docstrings remain ignored by the existing AST string-literal walker.
- No spec edits are required.
- No retired address terminology is introduced.

## Implementation

1. Expand raw admin carrier literal coverage from hosted-agent lifecycle only to the full SDK-owned Admin + Gateway ability set.
2. Add focused tests for hub, pairing, credential, session, and device revoke literals.
3. Run targeted and full Python SDK tests plus diff/terminology checks.
