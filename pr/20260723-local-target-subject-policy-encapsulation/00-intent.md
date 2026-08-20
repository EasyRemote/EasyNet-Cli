# Local Target Subject Policy Encapsulation

## Intent

Remove the product-visible `LocalAbilityTarget::default_subject_ura()` escape hatch.

`LocalAbilityTarget` should describe the selected local ability route: ability URA, dispatch name, and callee URA. It should not expose a reusable "default subject" that product adapters can pass around as tuple authority. Daemon-system calls may still derive a descriptor-bound subject, but that policy belongs behind a named daemon-system issuer.
