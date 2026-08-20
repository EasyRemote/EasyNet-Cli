# Decisions Log

- Decision: move only `identity.list_user_pubkeys`.
  Rationale: listing public-key trust rows is a read projection; registering a
  public key is a mutation and already has an explicit descriptor-bound subject.
