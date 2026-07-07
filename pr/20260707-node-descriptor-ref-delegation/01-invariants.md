# Invariants

1. DescriptorRef projection is owned by daemon/Axon identity helpers exposed
   through the injected transport.
2. Node must not parse DescriptorRef grammar or derive Ability URA by string
   splitting.
3. Node must not construct descriptor refs by local string concatenation.
4. Receipt fetch carriers must require and forward the supplied
   `descriptor_ref`; they must not fill or synthesize it.
5. Missing or malformed carrier fields fail before transport dispatch.
6. No non-URA naming or legacy input alias is introduced.
