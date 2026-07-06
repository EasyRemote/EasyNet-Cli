# Decisions Log

- Keep business subject classification in backend because it maps product user
  and hub subjects into descriptor-bound envelope subjects.
- Put only the canonical resource subject projection in the SDK so the backend
  can stop importing raw Axon helpers without moving product policy into the SDK.
- Replace the earlier package-level Go helper with
  `IdentityClient.DescriptorBoundResourceSubjectURA(ctx, ownerURA, path)`. A
  package function cannot satisfy the facade boundary because it has no daemon
  identity transport.
- Defer backend migration until the backend path can pass an `IdentityClient`.
  Reintroducing `realm + owner_id + path` string assembly in the SDK would make
  the SDK another URA grammar owner, which violates the Axon -> CLI -> SDK
  layering target.
