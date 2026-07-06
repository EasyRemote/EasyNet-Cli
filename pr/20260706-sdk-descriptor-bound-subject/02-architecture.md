# Architecture

This slice keeps two distinct identity-addressing seams:

```text
SDK resource addressing:
  owner_ura + path
    -> Identity build_ura(kind=resource)
    -> resource URA

backend descriptor-bound envelope subject mapping:
  owner_ura + descriptor path
    -> IdentityClient.DescriptorBoundResourceSubjectURA
    -> Identity build_ura(kind=resource)
    -> resource subject URA
```

`IdentityClient.ResourceURA` remains the transport-backed public SDK seam for
ordinary consumers. `IdentityClient.DescriptorBoundResourceSubjectURA` is only a
domain-named facade over the same transport-backed resource projection. The SDK
does not accept `realm` and `owner_id` fragments because doing so would make the
language binding responsible for URA grammar. Backend product policy may decide
which owner URA and descriptor path to submit, but canonical resource-subject
construction remains behind the Identity daemon/Axon boundary.
