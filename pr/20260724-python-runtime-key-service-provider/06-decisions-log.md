# Decisions Log

## 2026-07-24

- Decision: migrate Python key-service/keyring helpers to `providers.runtime` without aliases.
- Reason: compatibility aliases under `providers.easynet` would keep canonical managed signing coupled to a product namespace.
- Decision: rename `DaemonKeyringSignatureProvider` to `RuntimeKeyringSignatureProvider`.
- Reason: once the provider lives in `providers.runtime`, daemon-named public types become product-specific naming defects.
- Decision: keep `owner_ura` allowed for runtime provider custody while retaining runtime-event-specific owner lowering checks.
- Reason: signer custody must model key ownership explicitly; `owner_ura` is a canonical runtime ownership fact in this context, not an EasyNet product route concept.
