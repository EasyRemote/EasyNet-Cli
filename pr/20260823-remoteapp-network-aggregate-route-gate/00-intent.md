# RemoteApp network aggregate route gate

## Intent

Tighten the RemoteApp product-completion aggregator so NAT, direct, WebRTC
STUN, TURN relay, and EasyNet relay completion cannot be inferred from weak
`coverage=true` flags alone.

The dedicated `remoteapp-network-fallback-e2e.sh` verifier remains the owner of
live WebRTC, network fixture, candidate-pair, media, and credential-redaction
evidence. The aggregate completion gate now requires the verifier's report to
carry enough route-scenario summary to justify including that report in a
product-complete claim.

## Non-goals

- Do not duplicate the full network fallback verifier inside the aggregate gate.
- Do not claim live network fallback has passed without a real
  `remoteapp-network-fallback-e2e.sh` artifact.
- Do not alter RemoteApp plugin/runtime implementation paths in this slice.
