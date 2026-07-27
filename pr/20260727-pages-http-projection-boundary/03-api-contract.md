Public behavior:
- Static Pages HTTP requests still return the same status/body/header projection.
- Root path still maps to `/index.html`.
- HEAD still returns headers without body.

Internal contract:
- `serve_bytes(user, project_id, path) -> ServedBytes` remains the internal listener adapter function.
- `bytes_from_value(Value) -> Result<ServedBytes>` remains fallible and schema-bound.

Error contract:
- Invalid fetch projection returns status 502.
- Unpublished project returns status 503.
- Unknown or hidden file paths return status 404.
