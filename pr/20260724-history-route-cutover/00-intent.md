# Intent

## Goal

Remove the legacy route seam that allows `invocation.history.list` to be projected as a target-owned daemon-system remote ability. Receipt history is a canonical runtime receipt/session query, not a device-owned system action.

## Non-goals

- Do not weaken daemon admission authority checks.
- Do not add compatibility fallbacks for device-subject history reads.
- Do not synthesize descriptors for remote system abilities that are not present in the runtime realm catalog.
- Do not change the public `invocation.history.list` ability name or response shape.

## Acceptance Criteria

- Target-owned remote system dispatch rejects `invocation.history.*` selectors before constructing a daemon-target-owned subject.
- Receipt history remains owned by the canonical invocation history facade/query path.
- Architecture gates reject reintroducing direct target-owned history routing.
- Errors point callers to the receipt history path instead of reaching daemon admission with a mismatched session authority subject.
