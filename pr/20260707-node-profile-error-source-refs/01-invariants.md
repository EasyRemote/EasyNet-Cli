# Invariants

1. Profile provenance belongs in `SDKError.details`, not as new top-level wire
   schema fields.
2. Existing detail keys must be preserved; `source_ref` must not overwrite a
   caller- or daemon-provided value.
3. Source refs are stable package refs with the form
   `node_sdk.profile.<profile>`.
4. Error-class projection is derived only from canonical current error codes.
5. Profile errors remain typed `SDKError` values with canonical `INVALID_ARGUMENT`
   validation classification.
6. No non-URA naming or legacy input alias is introduced.
