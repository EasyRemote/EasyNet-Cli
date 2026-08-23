# Architecture

`federation discover` owns a small explicit read-scope state machine:

- `--operator-audit` -> operator/audit directory reader.
- `--user-id <id>` -> explicit User directory reader.
- neither flag -> current credential-bound User directory reader.

The product facade owns this selection. The federation directory reader owns
the product read boundary, and `remote_invoke` owns the signed invocation tuple.
The operator tuple constructor additionally validates that the local daemon URA
is an Authority before descriptor resolution or daemon I/O.

No Device-to-Authority fallback is introduced.
