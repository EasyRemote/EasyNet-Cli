# Architecture

Runtime Core owns the invocation state machine:

`DraftImmutable -> Prepared -> Signed -> Submitted`

The existing `PreparedInvocation` already carries `SigningMaterial` with a
`SignerPolicy`. The architectural gap is that `SignedInvocation` dropped that
policy when transitioning from prepared to signed. That made the object
submit-ready without preserving the proof field required by the SPEC.

This slice exposes the daemon-owned local signing transition through C ABI:

`Prepared -> sign_prepared_local -> Signed`

Language C ABI transports do not sign locally when the signed envelope carries
`policy.mode = local_daemon_signing`; they ask libeasynet_cli to consume the
prepared handle through the local signing transition.
