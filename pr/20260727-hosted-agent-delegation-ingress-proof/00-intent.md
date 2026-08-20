# Hosted-Agent Delegation Ingress Proof

## Goal

Replace the hosted-agent delegation issuer's boolean local-ingress flag with an explicit ingress proof type so daemon-local authority cannot be represented as an unowned `true`/`false` compatibility switch.

## Non-goals

- Do not change public CLI, FFI, SDK, or ability handler behavior.
- Do not relax public signed admission.
- Do not add a fallback path for legacy hosted-agent metadata.

## Acceptance criteria

- Hosted-agent delegation materialization accepts an explicit ingress state rather than `bool`.
- Trusted-local materialization remains restricted to daemon-local system envelopes.
- Public signed and bootstrap ingress reject unsigned hosted-agent delegation request metadata.
- Existing unary, stream, bidi, and exact daemon route dispatch paths keep descriptor-bound behavior.
