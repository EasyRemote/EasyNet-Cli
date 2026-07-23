# API Contract

Public behavior:
- Missing subject: fails with `reason=subject_required`.
- Malformed URA subject: fails with `reason=subject_required` before table lookup.
- Non-resource URA subject: fails with `reason=subject_required` before table lookup.
- Unknown canonical resource URA: fails with `reason=resource_not_found`.
- Wrong resource type: fails with `reason=resource_type_mismatch`.

Internal contract:
- `is_resource_ura_subject` remains available for read-only permission probes, but delegates to the explicit classifier.
- No caller/callee/device/default fallback may be introduced.
