# Decisions Log

- 2026-07-16: Keep host filesystem abilities as Device system abilities.
- 2026-07-16: Keep content-addressed blob state user-owned, with daemon-native
  execution made explicit through a files executor Agent.
- 2026-07-16: Remove the remaining `alice.files.get`-style dereference path in
  favor of owner-local `files.get` under the explicit `<user>.files` authority
  root.
- 2026-07-16: Add `R31_FILE_RESOURCE_OWNERSHIP_FORK` to keep the split
  executable: Device-owned `fs.*`, user-owned owner-local `files.*`, and
  OpenAI compatibility as a Device facade that invokes the Files executor root.
