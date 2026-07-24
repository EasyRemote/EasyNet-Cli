# Architecture

There are two layers:

1. Axon generated protocol schema. This layer is controlled by Axon and is
   generated into this repository for providers.
2. Canonical SDK facade receipt JSON. This layer is owned by the SDK and must
   expose product-neutral runtime concepts.

This iteration changes layer 2 only. Provider projections map generated Axon
`SessionAuthority.backend_ura` / `user_ura` into SDK facade
`issuer_ura` / `subject_ura`. Validators and canonical-byte builders consume
only the generic facade names.

The canonical byte layout remains:

`tag || issuer_ura || subject_ura || session_id || scopes || audiences || issued_at_ms || expires_at_ms || signature`
