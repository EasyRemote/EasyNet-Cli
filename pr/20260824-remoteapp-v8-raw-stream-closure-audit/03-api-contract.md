# API contract

The v8 callback shape is:

```c
typedef void (*RuntimeInvocationStreamV8Callback)(
    void *user_data,
    const char *metadata_json,
    const uint8_t *payload,
    size_t payload_len
);
```

`metadata_json` must carry sequence, kind, state, terminal,
transport_terminal, payload_content_type, admission_receipt,
terminal_receipt and error. Data payload is raw bytes. Terminal/error frames may
have an empty payload. EOF remains one unambiguous all-null callback.

Feature discovery, not optimistic symbol calls, determines whether a binding
may select the v8 representation. ABI v7 remains the fallback representation.
