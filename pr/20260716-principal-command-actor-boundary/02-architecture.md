# Architecture

## Boundary

`src/cli/commands/groups/principal.rs` is a CLI facade over daemon-owned
PrincipalLifecycle abilities. It may lower ergonomic CLI arguments into ability
payloads, but it must keep actor-source policy visible before dispatch.

## Ownership

- CLI facade: selects `PrincipalCommandActor`.
- Principal command serializer: projects the selected actor into JSON.
- Daemon PrincipalLifecycle aggregate: validates canonical URA, proof,
  idempotency, version, and transition legality.

## Convergence

The old helper combined actor selection and command JSON construction. This
slice splits them so lifecycle actor source is a small explicit state machine:

- supplied actor URA;
- subject-self authorization.

That removes the hidden fallback without changing public payloads.
