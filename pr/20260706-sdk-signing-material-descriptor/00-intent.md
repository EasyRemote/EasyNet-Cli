# Intent

Remove SDK-side descriptor fallback from prepared signing material.

`SigningMaterial` is daemon/Axon-owned canonical material. The Go and Python
facades may decode and validate it, but they must not fill
`signing_material.descriptor_ref` from the Invocation tuple or any top-level
prepared field.
