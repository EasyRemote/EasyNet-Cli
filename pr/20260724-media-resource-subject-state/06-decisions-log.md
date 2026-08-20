# Decisions Log

- 2026-07-24: Selected media resource subject ingress because malformed/non-resource/resource states were internally collapsed to a boolean even though lifecycle semantics are distinct.
- 2026-07-24: Kept public `subject_required` behavior for malformed and non-resource subjects while making those states explicit internally.
- 2026-07-24: Added SPEC v2 coverage to forbid the old `parse_ura(...).unwrap_or(false)` bool-collapse pattern in this resolver.
