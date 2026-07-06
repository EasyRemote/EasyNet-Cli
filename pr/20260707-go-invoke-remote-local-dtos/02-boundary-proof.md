# Boundary Proof

Correct boundary:

```text
Go SDK caller
  -> EasyNet-Cli SDK local bridge DTOs
  -> internal conversion to Axon DTOs
  -> Axon marshal/unmarshal/validation helpers
```

Incorrect boundary:

```text
Go SDK caller
  -> public Axon type aliases
```

The local DTOs prevent product-facing SDK code from depending on Axon package
types. Delegated constants and helper calls prevent the SDK from becoming a
second protocol implementation.
