# Intent

Close the Go Publication AbilityImpl lifecycle gap by wiring complete lifecycle
requests through both the Runtime Core transport and the C ABI v4
carrier/projection functions that already exist.

This slice does not introduce a Go-side lifecycle protocol. Go remains a
facade: runtime transport delegates descriptor-ref construction to
`IdentityClient`, C ABI transport asks `libeasynet_cli` to build complete
Invocation carriers, and both submit through Runtime Core.
