# Intent

Goal: remove the duplicated runtime-signing policy derivation from the
integration test key-service fixture and make it consume the canonical daemon
identity policy reference.

Non-goals:

- Do not weaken key-service policy checks.
- Do not accept both retired and canonical policy namespaces.
- Do not change public LocalRuntime or SDK invocation behavior.

Acceptance criteria:

- The fixture no longer computes a local `daemon-key-inventory:*` policy ref.
- The fixture validates requests with `daemon::identity::signer_policy_ref`.
- The pages LocalRuntime integration test invokes through descriptor-bound
  signing successfully.
- Convergence gates prevent the retired fixture policy namespace from returning.
