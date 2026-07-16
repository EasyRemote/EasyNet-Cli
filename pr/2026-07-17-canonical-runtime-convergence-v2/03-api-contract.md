# API Contract

## Invocation JSON

Required fields:

- `caller_ura`
- `callee_ura`
- `descriptor_ref`
- `subject_ura`
- `nonce_base64`
- `causal_context`
- exactly one of `args` or `arguments_base64`

`causal_context` is a structured object with a declared form. An empty causal
context is legal only when represented explicitly by the caller or by a named
system derivation policy.

## Signing and Key Custody

- Public SDK code may prepare and submit invocation material, but it may not
  generate process-local signing authority as a fallback.
- Daemon-owned internal calls request signatures from daemon key custody.
- Test signing material is allowed only as an explicit test fixture.

## Receipt Construction

Receipt bodies require authority and proof facts. Missing values are
construction errors, not defaults. Public edge adapters may normalize released
wire shapes only by constructing complete descriptor-bound inputs.

## Lifecycle Operations

The shared lifecycle contract covers:

- `start`
- `dispatch`
- `stream_open`
- `bidi_open`
- `child_dispatch`
- `cancel`
- `deadline`
- `terminal_receipt`
- `restart_recover`

Each operation declares allowed source states, next/terminal state,
deadline owner, cancellation authority, bounded resource responsibility, and
receipt/event observability.

## Error Contract

Errors remain typed by runtime class: invalid argument, unavailable,
permission/admission denied, cancellation, deadline, unsupported, and internal
failure. Product-specific error text must not become a canonical SDK type.
