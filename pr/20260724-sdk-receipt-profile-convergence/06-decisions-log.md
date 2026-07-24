# Decisions

## DEC-1: Strict profile only

Receipt identity profile validation in Node, Swift, and Java must accept only
`axon-strict-v2`.

`opaque` remains valid as a description of SDK-held receipt facts or cursors,
but not as a URA profile. `axon-legacy-v1` is retired and must fail closed.
