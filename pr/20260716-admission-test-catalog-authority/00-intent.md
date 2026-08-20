Admission test catalog authority convergence

Goal

Remove the admission facade test helper's accidental second authority root for
`session.open`. The helper uses the system inventory catalog because it needs
both Hub-owned `self.echo` and the `session.open` runtime-admin descriptor, but
it must not attach a local test handler to descriptor-only carrier contracts
such as `session.open`. That carrier's descriptor already exists in the system
inventory under the template authority; adding an execution row invents a
second root or violates runtime-hosted authority checks.

Expected effect

- PrincipalLifecycle admission tests no longer create a second `session.open`
  authority root while installing test handlers.
- The system inventory profile remains usable as a descriptor baseline for
  mixed Hub/Device admission tests.
- Descriptor-only carrier contracts remain descriptor-only in admission tests.
- The production fail-closed invariant for name-only control-plane lookup with
  multiple authority roots remains intact.
- Runtime behavior and public APIs remain unchanged.
