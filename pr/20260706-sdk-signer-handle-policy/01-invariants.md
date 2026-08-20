# Invariants

- Do not modify `docs/spec/daemon-sdk-requirements-v1.md`.
- SDK language facades must not mint signer authorization policy.
- `project_signer_handle` must project only daemon inventory keys that match
  the requested key id.
- Inactive daemon keys must fail closed before a signer handle reaches
  language facades.
- The projected policy must include `policy_ref`, `inventory_owner_ura`, and
  `key_state`.
- Metadata `policy_ref` must match `policy.policy_ref` so facades can reject
  forged or mixed signer handles.
