# RemoteApp media aggregate scenario gate

## Intent

Tighten the RemoteApp product-completion gate so audio/video data-plane product
completion cannot be inferred from `baseline/degraded_network/backpressure`
coverage flags alone.

The dedicated `remoteapp-media-adaptation-e2e.sh` verifier remains the owner of
the live media artifact contract. This slice makes that verifier export the
minimal scenario summary needed by the product-completion aggregator, then
requires that summary before a product-complete claim can pass.

## Non-goals

- Do not duplicate the full media verifier in the aggregate gate.
- Do not claim host audio, codec, or degraded-network behavior without a real
  `remoteapp-media-adaptation-e2e.sh` artifact.
- Do not change RemoteApp runtime/media implementation in this slice.
