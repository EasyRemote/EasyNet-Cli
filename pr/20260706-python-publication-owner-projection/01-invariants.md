# Invariants

- Do not modify `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not construct, split, partition, or suffix-match `owner_ura` to infer a
  user.
- Keep metadata-based `owner_user` hints as catalogue facts, not URA grammar.
- Keep `_parse_ura_with_addressing()` as the single Publication path to
  Identity/Axon URA projection.
- If the owner projection lacks user components, `list_user()` must fail closed
  for that row.
- Tests must prove user-owned Agent rows are matched through projection
  components, and rows without components are not matched by URI text.
