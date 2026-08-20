# Boundary Proof

## Storage boundary

The storage boundary reads bytes and reports `Loaded` or `Missing`. It owns
strict JSON parsing and I/O errors, but it does not decide whether missing
state means a fresh agent, a corrupted home, or a write initializer.

## Read projection boundary

Read APIs such as latest-session, lifelong-session, list-session, and inventory
queries intentionally render missing state as an empty inventory because no
session has been recorded yet.

## Mutation boundary

Session writes initialize a new index only when the storage reader reports
`Missing`. Existing malformed indexes continue to block mutation.

## Public read API boundary

The public `load_index(agent) -> SessionIndex` shape remains a stable read
projection, but production internals move to explicit load-state helpers.
