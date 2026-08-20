# Decisions Log

- Keep Axon as the grammar source of truth; add only CLI-local projection
  helpers in `core::ura`.
- Do not keep `register_device_pubkey` or `runtime_trust` as generic URA realm
  parser owners. Runtime/admission callers now consume `core::ura::realm_from_ura`.
- Treat malformed resolved device URAs in `federation_probe` as explicit
  product errors instead of falling back to the local realm.
