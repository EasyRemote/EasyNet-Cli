# Boundary Proof

Runtime Core owns prepared Invocation transport and signature workflow. Axon and
the daemon own canonical signing bytes and their descriptor binding.

Correct flow:

```text
daemon prepare
  -> signing_material.descriptor_ref
  -> SDK SigningMaterial DTO
  -> SignatureProvider.sign(material)
```

Incorrect flow removed here:

```text
daemon prepare without signing_material.descriptor_ref
  -> SDK reads tuple.descriptor_ref
  -> SDK mutates SigningMaterial descriptor binding
```

The latter makes the language facade a second source for canonical material
shape. Requiring the descriptor inside `signing_material` keeps the SDK as a
strict decoder/facade.
