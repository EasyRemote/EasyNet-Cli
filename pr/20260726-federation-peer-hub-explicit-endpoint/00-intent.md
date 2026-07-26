# Federation peer hub explicit endpoint cutover

## Goal

Remove the join-time peer-hub endpoint guess that converts device pairing
endpoints such as `axon://host:50051` or `http://host:50051` into
`https://host:50443` for `[daemon.federated_peers]`. A peer hub endpoint is a
runtime topology fact and must come from an explicit `--peer-hub` operator
argument or from a pairing credential endpoint that is already a canonical
TLS peer-hub endpoint.

## Non-goals

- Do not change the device-to-hub `[daemon].hub_endpoint` projection.
- Do not change daemon-config TOML materialization.
- Do not add another compatibility input or topology heuristic.

## Acceptance criteria

1. Explicit non-empty `--peer-hub https://host:port` remains accepted.
2. Blank `--peer-hub` is rejected instead of falling through.
3. Missing `--peer-hub` accepts only an already-`https://` pairing endpoint.
4. Missing `--peer-hub` with `axon://`, `http://`, or bare endpoints fails
   before writing `[daemon.federated_peers]`.
5. SPEC v2 rejects future reintroduction of guessed peer-hub endpoints.
