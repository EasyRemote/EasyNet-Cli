# API contract

## Constructor

`runtime_state_read_subject_ura(realm, user_id)` / language-idiomatic equivalent.

Input rules:

- `realm` must be non-empty after trimming.
- `user_id` must be non-empty after trimming.
- `user_id` must not be all-zero UUID.

Output:

- A canonical Resource URA owned by `user.<user_id>` with path `runtime-state/read`.

Errors:

- Invalid argument for malformed or all-zero inputs.

## Authority relation

A session authority for user `<user_id>` admits the runtime-state read subject because the Resource URA owner is exactly `user.<user_id>`.

It does not admit:

- target device URAs;
- all-zero placeholder user subjects;
- path-substring subjects that merely contain a user-owned path.
