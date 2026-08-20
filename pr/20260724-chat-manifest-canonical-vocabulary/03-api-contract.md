# API contract

Request:

- `prompt` remains required.
- Existing optional fields remain optional.

Response:

- `reply` remains required and typed as `string`.
- Existing response fields remain unchanged.

Error and tenant rules:

- No change to validation, routing, subject handling, authority, or receipt behavior.

Compatibility posture:

- Public behavior is preserved.
- Active manifest wording no longer presents compatibility history as part of the runtime model.
