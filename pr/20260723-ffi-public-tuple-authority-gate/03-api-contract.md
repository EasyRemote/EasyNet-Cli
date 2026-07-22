# API Contract

No exported C ABI symbol changes.

`easynet_invocation_invoke` continues to accept the same JSON object shape. Invalid tuples now return `ERR_INVALID_ARG` before daemon I/O when:

- any tuple URA is non-canonical;
- any tuple URA contains the all-zero placeholder principal;
- authority metadata is malformed;
- delegation authority subject does not match the envelope subject;
- session authority does not admit the envelope subject.
