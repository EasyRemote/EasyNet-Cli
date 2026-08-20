# Intent

`federation.join` can accept a verified principal enrollment proof and persist a
device-to-user ownership fact in runtime trust. That persisted
`TrustedPrincipalOwner` already carries both the canonical owner user id and an
optional routing alias used by hosted Agent publication.

This slice removes the join-path fork where the proof-bound owner persisted the
canonical user id but dropped the alias field, leaving later user-hosted Agent
publication without the owner alias needed for admission.
