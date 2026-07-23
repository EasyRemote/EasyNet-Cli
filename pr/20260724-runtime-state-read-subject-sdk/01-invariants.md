# Invariants

1. Runtime-state read subject is a user-owned Resource URA:
   `easynet:///r/<realm>/resource/user.<user_id>/runtime-state/read`.
2. Missing, blank, or all-zero user id fails before any daemon/device fallback can be selected.
3. The read subject is not the ledger filter. Device/ability/subject filters remain provider arguments after admission.
4. Session authority admission remains exact: authority issuer, callee, audience, scope, and subject ownership must all hold before the receipt/history provider is called.
5. The constructor is product-neutral. It names a generic runtime-state read resource, not EasyNet history, EasyRemote hub state, or product receipt pages.
6. Language SDKs must not diverge on subject projection rules.
