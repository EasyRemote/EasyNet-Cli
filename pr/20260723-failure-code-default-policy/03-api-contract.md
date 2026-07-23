# API Contract

## Internal Rust API

- `FailureCodeClassifier::classify_or_default(reason, default_code)`
- `FailureCodeClassifier::explicit_or_reason_default(explicit, reason, default_code)`
- `FailureCodeClassifier::normalize_or_default(candidate, default_code)`

## Public behavior

No public command output, receipt field, wire payload, or SDK API changes.

## Error behavior

Empty or unclassifiable inputs return the normalized caller default code.
