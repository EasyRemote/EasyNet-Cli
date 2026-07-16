# Invariants

- Retired identifier fixtures may contain `URI` or `_uri` only as failing
  examples.
- Gate output and fixture descriptions use `retired address-token` or
  `non-URA` vocabulary, not retired migration-era architecture language.
- Transport locator types such as `hyper::Uri` remain allowed when they model
  HTTP/gRPC endpoints rather than semantic identities.
- Existing guard behavior does not weaken: retired Node SDK names and system
  ability aliases still fail their self-tests.
- No product runtime, SDK public API, or daemon behavior changes in this slice.
