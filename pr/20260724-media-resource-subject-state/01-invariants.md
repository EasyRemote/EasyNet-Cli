# Invariants

- Media abilities must not accept `subject` from JSON args.
- Media abilities must not fall back from resource subject to caller, callee, device, or local default subject.
- Malformed URAs and non-resource URAs must fail before resource table lookup.
- Resource table load failures must not be projected as resource-not-found.
- The resolver owns subject ingress classification; individual media handlers should not reimplement subject parsing.
