# v8 raw stream release packaging

## Invariant

The ABI v8 raw-stream entry point is an additive transport representation for
high-payload RemoteApp/EasyRemote streams. If the checked-in header exposes
`runtime_invocation_stream_open_v8`, every release-shape package must also ship
the v8 export allowlist so downstream SDKs and operators can verify the exact
published symbol contract.

## Boundary proof

- `runtime_abi_version()` remains `7`; v8 is feature-detected through
  `runtime_feature_discovery` and the published v8 allowlist.
- The v8 release artifact is still generic runtime ABI metadata. It does not
  add product-specific RemoteApp symbols.
- Packaging and install scripts may copy ABI contract files, but they do not
  own Invocation, stream lifecycle, or RemoteApp session semantics.

## Change

- Ship `include/easynet_cli.exports.v8` beside the existing v7 allowlist in
  release tarballs and platform packages.
- Install the v8 allowlist with the header and v7 allowlist.
- Make release-install E2E verify v8 is sorted, has exactly 57 runtime symbols,
  contains all v7 symbols, and adds only `runtime_invocation_stream_open_v8`.

## Product effect

RemoteApp/EasyRemote raw media stream consumers can rely on the release package
to carry the same ABI extension contract that the SDK and source gates validate.
This closes a distribution seam; it does not by itself prove codec negotiation,
network relay, host audio, or cross-device RemoteApp E2E readiness.
