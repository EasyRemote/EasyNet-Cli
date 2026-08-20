# API Contract

Go:

- `IdentityClient.ResourceURA(ctx, ownerURA, path string) (string, error)`
- `IdentityClient.DescriptorBoundResourceSubjectURA(ctx, ownerURA, path string) (string, error)`

Python:

- `AddressingClient.descriptor_bound_resource_subject_ura(owner_ura: str, path: str) -> str`
- `IdentityClient.descriptor_bound_resource_subject_ura(owner_ura: str, path: str) -> str`
- `descriptor_bound_resource_subject_ura(owner_ura: str, path: str, *, library_path=None, control_path="") -> str`

All forms delegate to Identity `build_ura(kind=resource)`. The SDK does not
validate or join URA path segments locally beyond the generic transport request
shape; Axon owns canonical URA grammar and the daemon/Axon identity boundary
owns projection validity.
