# Node Prepare Sign Seam Boundary Proof

## Ownership

Runtime Core owns prepare/sign/submit object-state transitions. Axon/daemon
transport owns canonical bytes. The Node SDK owns typed DTO validation,
immutability boundaries, and caller-signature envelope projection.

## Call Path

```text
InvocationDraft
  -> RuntimeClient.prepare(...)
  -> injected RuntimeTransport.prepare(...)
  -> PreparedInvocation + SigningMaterial
  -> PreparedInvocation.signWithCallerSignature(...)
  -> SignedInvocation
  -> RuntimeClient.submitSigned(...)
```

## Rejected Designs

- Building canonical bytes in Node: rejected because Axon owns canonicalization.
- Allowing `PreparedInvocation` through submit: rejected because it collapses
  Prepared and Signed states.
- Accepting already-shaped anonymous objects in `submitSigned`: rejected because
  it weakens the object-state boundary.
