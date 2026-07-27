Public ingress contract:
- Required fields: `query_name`, `qtype`, `caller_ura`, `subject_ura`, `realm_hint`, `ability_name`.
- `ability_name` accepts either:
  - `null`: explicit no separate ability selector.
  - non-empty string: explicit owner-local/descriptor/Ability selector.

Rejected:
- Missing `ability_name`.
- Empty string ability selector.
- CamelCase aliases.
- Non-canonical caller/subject URAs.

Compatibility stance:
- This intentionally removes the missing-field compatibility path.
- Public descriptor schema is updated so new callers know the explicit nullable selector contract.
