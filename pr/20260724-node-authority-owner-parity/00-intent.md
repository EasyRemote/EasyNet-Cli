# Intent

Goal: remove a Node SDK authority-model parity gap by projecting the same canonical session owner fields exposed by Go and Python.

Non-goals:
- Do not add product-specific EasyNet or EasyRemote lifecycle semantics.
- Do not change daemon authority metadata wire shape.
- Do not preserve a compatibility-only alternate authority model.

Acceptance criteria:
- Node `SessionAuthority` and `SessionAuthorityRequest` expose canonical owner/principal URA fields consistently with Go/Python.
- Optional canonical owner fields are validated and normalized without leaking onto the staged daemon wire when absent.
- SPEC v2 and SDK conformance gates continue to pass.
