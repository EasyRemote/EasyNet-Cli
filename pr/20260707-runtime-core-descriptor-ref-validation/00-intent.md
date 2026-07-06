# Runtime Core DescriptorRef Validation

## Goal

Reject descriptor-bound Invocation drafts that do not expose a valid descriptor ref shape before runtime dispatch.

## Non-goals

- Do not add product-specific descriptor names.
- Do not replace daemon/Axon canonical descriptor-ref projection.
- Do not introduce descriptor-ref aliases or legacy input forms.

## Acceptance Criteria

- Go `InvocationBuilder.Build` validates descriptor refs through the existing Axon-delegated helper.
- Python `InvocationBuilder.build` rejects missing descriptor version, non-ability URA, and ambiguous `@` forms through a generic SDK seam.
- Public DTO fields remain unchanged.
- Full Go and Python SDK tests continue to pass.
