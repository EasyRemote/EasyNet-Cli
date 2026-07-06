# Architecture

Runtime Core owns the invocation state machine:

`DraftImmutable -> Prepared -> Signed -> Submitted`

The existing `PreparedInvocation` already carries `SigningMaterial` with a
`SignerPolicy`. The architectural gap is that `SignedInvocation` dropped that
policy when transitioning from prepared to signed. That made the object
submit-ready without preserving the proof field required by the SPEC.

This slice fixes the state object first. Live daemon keyring execution can then
plug into the same transition and return a `SignedInvocation` with the same
policy-proof shape.
