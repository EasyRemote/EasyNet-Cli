## Request contract

Daemon start accepts:

- an explicit `HOME` in `DaemonStartConfig.env`; or
- the process `HOME` environment variable.

The explicit child `HOME` wins only when non-blank.

## Error contract

Missing or blank home roots fail with a typed daemon lifecycle error:

`DaemonError::DaemonHomeUnavailable { source }`

No path projection should silently materialize `./.easynet`.

## Compatibility

The public Rust and FFI start APIs remain compatible because they already return
typed start errors. The change removes an internal fallback, not a public
parameter.
