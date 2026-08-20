# Decisions

1. `PreparedInvocation` is produced only by `RuntimeClient.prepare` over the
   injected Runtime transport. Node does not construct canonical signing bytes.
2. `SignedInvocation` is the only submit-ready object accepted by
   `RuntimeClient.submitSigned`.
3. Caller signatures are attached as already-produced signature material. Node
   preserves the signature fields and daemon signing material without rewriting
   either side.
4. Node intentionally does not declare local daemon signing conformance in this
   slice because signer-handle acquisition and daemon signing provider paths are
   not implemented.
