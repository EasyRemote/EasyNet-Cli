# FFI Public Tuple Authority Gate

## Intent

Move canonical tuple and authority-shape rejection to the FFI public ingress before daemon transport.

The product-facing C ABI already requires a complete invocation tuple, but it only checked field presence. Invalid all-zero principals and authority metadata subject mismatches could still cross into daemon admission, where they surfaced as late `AUTHORITY_SUBJECT_MISMATCH`, caller signer, or descriptor route errors.

This change must not synthesize a replacement subject and must not add a compatibility path. It should fail closed before dispatch when the public tuple contradicts canonical authority metadata.
