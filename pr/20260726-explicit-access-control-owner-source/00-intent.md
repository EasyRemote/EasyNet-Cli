# Intent

## Goal

Remove the legacy access-control check behavior that silently defaulted a missing `owner_source` to `Subject`.

## Non-goals

- Do not add product-specific EasyNet or EasyRemote policy semantics to the SDK.
- Do not preserve a compatibility path for omitted `owner_source`.
- Do not change public ability names or receipt/admission public interfaces.

## Acceptance criteria

- `authority_binding.check` requires explicit `owner_source` at the daemon boundary.
- Go and Python SDK access-control clients reject missing `owner_source` before dispatch.
- The daemon schema advertises `owner_source` as required.
- Tests prove the missing-field negative path.
