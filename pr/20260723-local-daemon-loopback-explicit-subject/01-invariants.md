# Invariants

## Semantic invariants

- Invocation subject is a tuple fact, not an optional value recovered from the
  callee.
- Callee selection and subject selection remain separate responsibilities.
- Daemon-local root calls may use the daemon's published identity as subject,
  but the subject must be explicit before wire invocation construction.

## Safety invariants

- No branch may synthesize a product device/user subject when control discovery
  does not publish daemon identity.
- The key-service-backed `_system.local` caller remains the signer; this change
  does not introduce product-owned signing.
- Invalid or absent daemon identity must fail closed before admission.

## Boundedness invariants

- There is one tuple construction path for generic local loopback RPC.
- There is no retry, fallback, or legacy control-socket path added by this
  refactor.
