# Intent

Remove unsupported browser ability vocabulary from canonical routing core tests.

`browser.open_session` is not a provider-backed runtime capability in this
repository. Routing tests should exercise owner-kind semantics with neutral
runtime concepts, not product ability names that imply a shipped browser
surface.
