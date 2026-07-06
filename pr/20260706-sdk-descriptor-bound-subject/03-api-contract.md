# API Contract

Go:

- `IdentityClient.ResourceURA(ctx, ownerURA, path string) (string, error)`
- `DescriptorBoundResourceSubjectURA(realm, ownerID, path string) (string, error)`

Python:

- `AddressingClient.descriptor_bound_resource_subject_ura(owner_ura: str, path: str) -> str`
- `IdentityClient.descriptor_bound_resource_subject_ura(owner_ura: str, path: str) -> str`
- `descriptor_bound_resource_subject_ura(owner_ura: str, path: str, *, library_path=None, control_path="") -> str`

The Python forms delegate to Identity `build_ura(kind=resource)`. The Go
package helper is deliberately narrow and validates descriptor subject
segments before producing the resource-subject URA used by backend envelope
materialization.
