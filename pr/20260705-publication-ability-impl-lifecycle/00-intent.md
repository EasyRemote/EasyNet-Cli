# Intent

Implement the Publication profile AbilityImpl enable/disable SDK lifecycle contract through the Rust daemon SDK contract, C ABI v4 projection, and Python facade transport.

## Goal

- Replace the current Python C ABI `NOT_IMPLEMENTED` path for `PublicationClient.enable_ability_impl` and `disable_ability_impl`.
- Keep lifecycle mutation semantics owned by the daemon Publication contract.
- Preserve the existing Python public API shape while making it backed by complete daemon Invocation carriers.

## Non-Goals

- Do not invent a Python-side AbilityImpl state machine.
- Do not project read-model rows into mutation results.
- Do not implement plugin host execution, product decorators, or EasyRemote introspection.
- Do not redefine Axon URA, DescriptorRef, Invocation, receipt, or signing semantics.

## Acceptance Criteria

- C ABI exposes enable and disable carrier builders plus projection functions.
- Python `CABIPublicationTransport` executes enable/disable through Runtime Core and projects daemon output.
- Invalid incomplete IDs and non-Ability URAs fail before dispatch.
- Existing Publication list/show/deploy/unpublish behavior remains unchanged.
