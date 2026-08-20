# Invariants

- `owner_user_id` is never accepted as an access-control ability request input.
- Non-token principals use `principal_ura` as request input.
- Token principals use token-specific `token_id`; they do not require a
  generic principal scalar at the ability boundary.
- Daemon storage may keep scalar indexes, but they are derived after
  deserialization and are not SDK/provider responsibility.
- Go and Python provider facades must not send `owner_user_id` or
  `principal_id` in access-control ability arguments.
- Projection DTOs may still expose scalar fields for existing public API
  compatibility.
