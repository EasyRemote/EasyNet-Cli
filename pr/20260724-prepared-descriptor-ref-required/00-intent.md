# Intent

Remove the cross-language compatibility fallback that lets SDK `PreparedInvocation`
objects synthesize the top-level `descriptor_ref` from
`signing_material.descriptor_ref`.

The prepare response is part of the canonical runtime model. Its top-level
descriptor binding is a first-class attestation fact, not a convenience field.
When providers omit it, SDKs must fail closed instead of repairing the payload.
