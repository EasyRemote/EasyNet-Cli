## Intent

Close the remaining runtime-identity public-surface fork where Go exported
`ErrRuntimeIdentityNotFound` and `ErrRuntimeIdentityUnavailable` were counted
as canonical `runtime_identity` capability symbols even though they are source
compatibility aliases for the daemon key-service error owner.

## Expected effect

| Effect | Expected outcome |
| --- | --- |
| Architecture convergence | Runtime identity keeps one error owner: daemon key service. Runtime-named aliases cannot re-enter canonical capability evidence. |
| Architecture cleanliness | Public compatibility aliases are quarantined with explicit replacement metadata instead of being counted as runtime model capabilities. |
| Product acceleration | SDK reviewers can distinguish real runtime identity capability from legacy/source-compatible error names mechanically. |

## Non-goals

- Do not remove the Go exported aliases in this slice; that is a public API
  removal and belongs to an explicit major-version cutover.
- Do not alter key-service transport behavior or signing semantics.
