# Decisions Log

- Treat descriptor catalog probe subject selection as a closed runtime state
  decision instead of as a procedural helper.
- Do not add compatibility behavior for remote system abilities; the existing
  negative tests that require caller signer / route authority remain load-bearing.
