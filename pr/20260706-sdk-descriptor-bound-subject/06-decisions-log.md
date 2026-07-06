# Decisions Log

- Keep business subject classification in backend because it maps product user
  and hub subjects into descriptor-bound envelope subjects.
- Put only the canonical resource subject projection in the SDK so the backend
  can stop importing raw Axon helpers without moving product policy into the SDK.
- Do not add a Go `IdentityClient.DescriptorBoundResourceSubjectURA` alias:
  it creates another public method without new behavior. Python keeps the name
  as a consumer-facing convenience over the existing addressing facade.
