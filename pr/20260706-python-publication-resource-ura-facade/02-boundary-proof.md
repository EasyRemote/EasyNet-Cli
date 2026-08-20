# Boundary Proof

Publication owns package/resource reference projection, not URA grammar.

The only valid resource URA build flow in Python Publication is:

```text
local filesystem path
  -> Publication relative resource path facts
  -> AddressingClient.resource_ura(owner_ura, path)
  -> ResourceRef DTO
```

If a custom addressing object only supports `resource_ura(realm, owner_id,
path)`, it is an old facade shape and must fail. Supporting it would require
Publication to know how to form owner ids, which duplicates Identity/Axon
grammar.
