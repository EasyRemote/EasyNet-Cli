# Invariants — Hub API Readiness Gate

1. The Hub API readiness preflight is read-only. It must not start Docker,
   mutate credentials, pair devices, restart daemon processes, or repair Hub
   state.
2. A runtime-status failure or missing `hub_api_endpoint` is a first-class
   failed preflight, not an unstructured shell abort.
3. Failed preflight artifacts must preserve:
   - `runtime_status`
   - `connection_state`
   - `connection_failure`
   - `hub_endpoint`
   - `hub_api_endpoint`
   - `preflight_error`
4. The product-flow harness must execute Hub API readiness before daemon,
   frontend, capture, media, or input evidence.
5. A failed Hub API or credential-verification gate is evidence against product
   completion. It must not be reinterpreted as RemoteApp success or skipped by a
   later local-provider test.
