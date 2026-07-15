# Intent

Converge the RFC-014 access-control ability input boundary on URA-owned
identity. Public ability requests should carry `owner_ura` and
`principal_ura` or token-specific `token_id`; daemon-private storage keys such
as `owner_user_id` and `principal_id` are derived inside the daemon and remain
only persistence/projection facts.

This slice targets the A30 root fork: SDK provider facades and daemon ability
descriptors were exposing storage keys as request input, creating two identity
models at the mutation/read boundary.
