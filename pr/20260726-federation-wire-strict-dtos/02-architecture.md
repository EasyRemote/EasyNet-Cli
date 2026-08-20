# Architecture

```text
Axon Invocation bytes
  -> daemon federation wire DTO
  -> wrapper/read-model validation
  -> PresenceRegistry / directory view / resolver response
```

The DTO layer must be a strict schema gate. Compatibility with stale product
fields belongs nowhere in this path.
