# Boundary Proof

## Ownership

Axon owns canonical Invocation assembly, descriptor resolution, signature
verification, replay checks, admission, terminal receipt proof facts, and
stream/bidi lifecycle semantics.

EasyNet-Cli owns daemon lifecycle, key-service custody, device/Hub policy,
plugin/provider execution, MCP, EAL/Mission orchestration, schedules, pages,
media, and route locality decisions. Those decisions must enter Axon through a
complete descriptor-bound invocation request.

## Admissible CLI Responsibilities

- Build typed daemon policy inputs.
- Resolve local product state needed to choose caller/callee/ability/subject.
- Request system signatures from daemon key custody for `_system.local`.
- Forward complete Invocation material into Axon-owned builders.
- Persist daemon projections and product diagnostics.

## Forbidden CLI Responsibilities

- Public plain canonical-byte signing or verification.
- Process-local signer generation as authority fallback.
- Ad hoc receipt proof fact synthesis.
- SDK receipt constructors, encoders, and JSON parsers that synthesize missing
  authority or proof facts.
- Ability route handlers that bypass LocalRuntime.
- Silent defaults for `subject` or `causal_context` at public ingress.
- Product feature families exported as SDK canonical runtime abstractions.

## State Machine Proof Obligations

Every invoke, stream, and bidi path must have one terminal state:

```text
Prepared -> Admitted -> Running -> Succeeded
Prepared -> Rejected
Prepared -> Admitted -> Running -> Failed
Prepared -> Admitted -> Running -> Cancelled
Prepared -> Admitted -> Running -> DeadlineExceeded
```

Terminal states must emit a receipt or typed terminal event. Cancellation and
deadline paths must be idempotent and replay-queryable.

## URA Vocabulary

Active source, schemas, tests, errors, and normative docs use URA. `uri` may
appear only as a transport library API term for HTTP request paths or in
historical docs explicitly outside active gates.
