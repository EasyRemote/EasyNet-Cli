# SDK-Owned EasyRemote Stream Values

## Objective

Move EasyRemote stream frame projection out of `easyremote.client.Stream` and
into the Python SDK Runtime Core transport facade. EasyRemote should expose the
Python iterator/product UX, while SDK Runtime Core owns daemon stream frame
receive, idle timeout mapping, terminal-frame handling, payload projection, and
wire error projection.

## Boundary

- SDK owns daemon stream frame interpretation for the EasyRemote transport
  facade: envelope errors, host-stream error payloads, JSON/null/bytes payload
  projection, idle timeout, and close-on-exhaustion.
- EasyRemote owns public exception vocabulary, decorator/client ergonomics, and
  product method names.
- This slice does not change Axon stream protocol semantics or the daemon SDK
  requirements spec.

## Non-goals

- Do not implement full schema-backed stream terminal receipt events.
- Do not change bidi session semantics.
- Do not change EasyRemote public `Stream` iteration behavior.
