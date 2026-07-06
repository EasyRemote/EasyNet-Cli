# Invariants

1. Shared conformance manifests are the SDK behavior contract for shipped
   language facades.
2. Action-adapter reports are not sufficient as JSON files; they must be
   validated by the runner against declared cases, profiles, language ownership,
   and repository-local evidence paths.
3. Rust and C ABI reports remain core projection evidence. Go and Python reports
   remain P0 facade evidence. Node report remains P1 seam evidence only.
4. Product smoke tests prove consumer health, but they do not replace SDK
   conformance report closure.
5. The aggregate cutover-readiness gate must fail if any shipped report drifts
   from the shared manifest.
