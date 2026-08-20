# Invariants

- AbilityImpl lifecycle mutations must lower to complete daemon Invocation
  carriers before Runtime Core execution.
- Go must not hand-build `ability.impl.enable` or `ability.impl.disable`
  descriptor refs, args, or projection DTOs.
- Returned lifecycle records must come from C ABI projection functions.
- Transport close, handle ownership, and dynamic symbol binding must follow
  the existing `CABIPublicationTransport` object model.
- Existing Go and Python Publication public APIs must remain stable.
