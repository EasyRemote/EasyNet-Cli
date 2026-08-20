# Node DescriptorRef Delegation Intent

Add Node/TypeScript evidence for the shared DescriptorRef helper delegation
case in `docs/spec/daemon-sdk-requirements-v1.md`.

## Scope

- Pin that `IdentityClient.projectDescriptorRef`,
  `canonicalAbilityDescriptorRef`, and `abilityURAFromDescriptorRef` delegate to
  the injected identity projection transport.
- Pin that receipt fetch carriers consume caller-provided `descriptor_ref`
  without facade-side descriptor construction.
- Declare Node for `invocation/descriptor_ref_helper_delegation` only with test
  evidence.

## Out Of Scope

- No local DescriptorRef grammar.
- No local ability descriptor concatenation helper.
- No daemon or C ABI transport provider for Node.
