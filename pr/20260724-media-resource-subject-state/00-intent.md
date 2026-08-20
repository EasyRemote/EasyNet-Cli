# Intent

Goal: remove the bool-collapse seam in media resource subject resolution by modeling resource subject ingress as an explicit state projection.

Non-goals:
- Do not add new media capabilities.
- Do not change public ability names or descriptor surfaces.
- Do not introduce product-specific EasyRemote or browser lifecycle into the shared resource subject resolver.

Acceptance criteria:
- Resource subject validation distinguishes missing, malformed URA, non-resource URA, and canonical resource URA internally.
- Public handler errors remain compatible: invalid or non-resource subjects continue to fail closed with `subject_required`.
- Resource table lookup only runs after the subject is proven to be a canonical resource URA.
- SPEC/gate coverage prevents reintroducing `parse_ura(...).unwrap_or(false)` bool collapse in the resource subject resolver.
