# Decisions log

- 2026-07-24: Treat product-side `subject_ura = callee_ura` for runtime-state/history reads as a missing SDK model, not as an admission bug. Admission must stay strict; construction should move to the SDK.
- 2026-07-24: Added the constructor across Go, Python, and Node instead of only fixing CLI Rust so the SDK remains one shared runtime model rather than three language-specific subject policies.
- 2026-07-24: Kept device-subject history requests as explicit negative tests; the constructor does not broaden admission, it prevents product consumers from building the wrong tuple.
