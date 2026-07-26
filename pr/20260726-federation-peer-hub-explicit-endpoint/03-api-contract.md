# API contract

## Inputs

- `--peer-hub`: optional, but when present it must be non-empty and start with
  `https://`.
- `Credentials.hub_endpoint`: required by pairing. It may remain the
  device-to-hub endpoint for `[daemon].hub_endpoint`. It can be used for
  `[daemon.federated_peers]` only when it already starts with `https://`.

## Outputs

- On success, `[daemon.federated_peers].<realm>` receives the resolved TLS
  peer-hub endpoint.
- On failure, no daemon-config write occurs.

## Errors

- Blank `--peer-hub`: `--peer-hub must not be empty`.
- Non-TLS explicit peer hub: `--peer-hub must be an https:// endpoint`.
- Ambiguous pairing endpoint without explicit peer hub: `federated_peers
  auto-wire requires --peer-hub when pairing hub_endpoint is not https://`.
