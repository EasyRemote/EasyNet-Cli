# Intent — RemoteApp Product-Flow Preflight Diagnostics

RemoteApp product completion must be proven by live product-flow evidence, not
by source-contract gates alone. The current live flow is blocked before
RemoteApp capture/media/input execution because Hub API and daemon credential
verification readiness are not green.

This change tightens the first upstream product gate:

- failed Hub API readiness preflights must still write a standard report;
- runtime connection state, credential failure, paired Hub endpoint, and missing
  Hub API endpoint must be preserved in evidence;
- frontend product-flow gates must reject a preflight that hides those fields.

The scope is diagnostic and evidence-contract hardening only. It does not claim
RemoteApp product completion.
