# Intent

Remove the stream/bidi runtime event alias that lets SDKs accept `event` when
the canonical frame field is `kind`.

This slice closes one legacy wire-shape fork: transport adapters and domain
decoders must require the same canonical field instead of repairing stale
callback frames inside the SDK.
