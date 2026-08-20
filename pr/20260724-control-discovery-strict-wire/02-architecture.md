# Architecture

`control.json` is the daemon's local attach boundary. It is not a product
read-model and not a backwards-compatible configuration file. Treating missing
fields as empty strings or zeroes allows stale or old daemon output to look
parseable, then fail later as route/readiness ambiguity.

The correct boundary is:

- daemon writes a complete strict discovery object;
- SDK decodes it as strict provider output;
- runtime connector decides whether the parsed discovery advertises an
  invocation endpoint;
- product UI renders Pages URLs only from an explicit daemon-bound port.
