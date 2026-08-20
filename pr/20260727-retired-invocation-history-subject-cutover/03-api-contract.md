Request contract:
- Public invocation requests must carry canonical `subject_ura`.
- Invocation-history read requests must use `easynet:///r/<realm>/resource/user.<user_id>/runtime-state/read`.

Error contract:
- All-zero principal placeholders produce all-zero identity validation errors.
- Retired invocation-history carriers produce retired-subject errors.
- Non-retired session subjects remain governed by session owner/session id validation.

Tenant rules:
- Realm stays encoded in the URA.
- No tenant inference or fallback is introduced.
