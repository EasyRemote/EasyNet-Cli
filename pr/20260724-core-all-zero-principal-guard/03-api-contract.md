# API Contract

## Core identity guard

- `ALL_ZERO_PRINCIPAL_ID` is the only Rust production constant for the placeholder value.
- `is_all_zero_principal_id(value)` checks exact trimmed principal IDs.
- `contains_all_zero_principal_placeholder(value)` checks embedded placeholder occurrences inside URAs or authority payload fields.

## Callers

Callers must not duplicate the constant and must not inline lower-case contains logic.
