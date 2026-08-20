# Federation Discover Daemon Identity and Local Presence

## Goal

Converge `federation.discover` onto daemon-owned identity and directory state.

## Concrete Use Case

When a CLI or SDK helper asks the running local daemon for `federation.discover`,
the signed loopback invocation must target the daemon's actual product identity:
Hub daemons use the Hub URA from control discovery, while device/both daemons use
their device URA. The discover response must also include local presence-registry
devices for the daemon realm, not only remote federated directory entries.

## Expected Effect

- Architecture convergence: daemon loopback calls no longer collapse Hub mode to
  the unpaired local device fallback.
- Product correctness: local devices present in the daemon registry are visible
  through `federation.discover` even when no remote federation view exists.
- Proof-chain cleanup: Hub discover loopback uses the `federation.discover`
  ability subject instead of pretending the Hub is a device subject.
