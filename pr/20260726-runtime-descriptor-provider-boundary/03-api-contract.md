Public ABI remains:

```c
int runtime_resolve_descriptor_ref(RuntimeHandle handle, const char *request_json, char **out_descriptor_json);
```

JSON request remains:
- `callee_ura`: required string.
- `ability`: required string.
- `call_mode`: required string.
- `provider`: optional string; supported values remain `ability_descriptor` and `receipt_history`.
- provider-specific `subject_ura` validation remains fail-closed.

JSON response remains:
- `descriptor_ref`
- `ability_ura`
- `owner_ura`
- `name`
- `call_mode`
- `source`

Error contract:
- ABI maps typed provider errors without substring classification at the FFI business boundary.
- Key-service custody internals stay redacted in runtime-owner-unavailable messages.
