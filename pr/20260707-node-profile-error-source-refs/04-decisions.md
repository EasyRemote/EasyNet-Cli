# Decisions

1. Node source refs use `node_sdk.profile.<profile>` to match the Go/Python
   package-source-ref pattern.
2. `profile` and `source_ref` remain in `SDKError.details`; the shared daemon
   error schema is unchanged.
3. Node now exposes error class, profile, and source-ref accessors on `SDKError`
   as language-facade ergonomics over canonical error codes and details.
4. Publication validation errors add stable machine reasons where Go/Python
   already expose the same reason semantics.
