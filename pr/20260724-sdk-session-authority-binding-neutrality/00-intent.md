# Intent

Remove product-named `backend_ura` / `user_ura` session-authority binding fields
from the SDK facade receipt model.

Axon generated protocol bindings still contain those field names. The SDK
facade does not need to expose them. It can project the same canonical authority
bytes through generic runtime names:

- `issuer_ura`
- `subject_ura`

This keeps proof hash byte order stable while removing product/topology naming
from the public SDK receipt proof-fact JSON model.
