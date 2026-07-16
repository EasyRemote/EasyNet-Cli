# Stateless Signing Material Decoder

## Intent

Restore the canonical SDK stateless external-signing path used by browser
prepare flows.

## Root Fork

The material-only prepare contract correctly omitted the native `prepared_id`,
but both SDK facades decoded that response as a retained prepared capability.
The retained decoder correctly rejected the omitted id, making the documented
stateless flow impossible.

## Boundary

`PreparedInvocation` owns a retained native capability and must require an id.
`SigningMaterial` owns the stateless caller-signing projection and must never
invent or retain a native prepared capability.
