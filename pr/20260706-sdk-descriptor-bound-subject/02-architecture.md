# Architecture

This slice keeps two distinct identity-addressing seams:

```text
normal SDK resource addressing:
  owner_ura + path
    -> Identity build_ura(kind=resource)
    -> resource URA

backend descriptor-bound envelope subject mapping:
  realm + descriptor owner id + descriptor path
    -> DescriptorBoundResourceSubjectURA
    -> resource subject URA
```

`IdentityClient.ResourceURA` remains the transport-backed public SDK seam for
ordinary consumers. `DescriptorBoundResourceSubjectURA` is intentionally a
package-level helper for backend envelope-subject materialization, where the
backend already owns the product decision to convert user or hub subjects into
descriptor-bound resource subjects.
