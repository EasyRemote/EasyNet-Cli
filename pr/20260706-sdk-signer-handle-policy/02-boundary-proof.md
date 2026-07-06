# Boundary Proof

Correct flow:

```text
daemon identity.list_user_pubkeys
  -> Rust identity_contract::project_signer_handle
  -> C ABI easynet_identity_project_signer_handle
  -> Go/Python SignerHandle validators
  -> Runtime Core prepare/sign/submit
```

The SDK projects provenance from daemon inventory facts. It does not authorize
signing from caller-provided handles alone.

Rejected flow:

```text
consumer provides key_id
  -> SDK accepts a signer handle without daemon inventory policy_ref
  -> local signer signs canonical material
```

That would turn signer handles into facade-local credentials. This slice keeps
the signer boundary daemon-owned.
