# Architecture

Root abstraction problem:

The user signing identity reconciler has two authorities:

- key custody inventory in the local key service;
- public trust registry state in the daemon runtime.

The read side of the trust registry was still using the generic local invoke
transport helper. That couples a signer lifecycle read to the daemon-self
shortcut and weakens the subject model precisely where signer custody needs the
least ambiguity.

Refactoring:

- `LocalUserPublicKeyRegistry::contains` uses `LocalRuntimeStateReadIssuer`.
- `LocalUserPublicKeyRegistry::register` keeps `LocalDaemonSystemAbilityIssuer`.
- The runtime-state read boundary gate now covers this file.
