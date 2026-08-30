# Invariants

1. Hydration must not change lifecycle state, state code, transition,
   interrupted transition, failure code, failure message, source, or observed
   timestamp.
2. Hydration may only fill missing transport/API context:
   - missing `hub_endpoint`
   - missing `hub_api_endpoint`
3. Hydration may only use credentials when the snapshot and credentials refer
   to the same `realm` and `node_id`.
4. A snapshot for a different device, realm, or host endpoint must remain
   unchanged so stale failure evidence is not contaminated by a later pairing.
5. The operation is a read-time projection improvement; it must not mutate
   credentials, start daemon processes, pair devices, or change RemoteApp plugin
   behavior.
6. Once endpoint context is available, a failed Hub API health probe must still
   write a standard report artifact instead of exiting through `set -e`.
