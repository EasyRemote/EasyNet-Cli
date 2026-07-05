# Receipt History C ABI Plan

## Objective

Close the Receipt profile gap where Go C ABI transport cannot build or execute
daemon `invocation.history.list`, `invocation.history.get`, and
`invocation.trace.get` carriers even though the daemon already owns those
abilities and the Go runtime facade already models the carrier state.

## Boundary Proof

- Axon owns receipt verification and canonical receipt semantics.
- EasyNet-Cli daemon owns the invocation history and trace read-model abilities.
- Rust daemon SDK contract owns SDK carrier construction for those daemon
  abilities.
- C ABI projects the Rust contract without exposing Axon protobufs or C handles
  to language facades.
- Go SDK calls the C ABI projection and Runtime Core invoke path; it must not
  construct receipt verification claims or reinterpret receipt hashes.

## Invariants

- Every built history carrier preserves the complete Invocation tuple: caller,
  callee, descriptor ref, subject, nonce, causal context, and args.
- Ability names are fixed in Rust via daemon governance ability constants.
- Request payloads are bounded JSON objects and reject unsupported fields.
- Runtime execution returns the daemon output JSON unchanged for history/trace
  read models.
- No cryptographic verification is claimed by this slice.

## Verification

- Rust FFI receipt tests for carrier construction and invalid handles.
- Go C ABI receipt tests for list/get/trace build and execution paths.
- Go SDK tests for affected package.
- Python SDK tests are not directly touched by this slice.
