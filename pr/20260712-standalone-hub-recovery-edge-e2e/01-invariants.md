1. Recovery proof replay is rejected by the daemon-owned PrincipalLifecycle
   aggregate, not by CLI-local prevalidation.
2. Deleted principal recovery is terminal on the live Hub daemon path.
3. Failed recovery attempts must not project new public keys into RuntimeTrust.
4. The test continues to use Hub URA TCP+TLS join with no Backend HTTP state.
5. Private key material remains absent from CLI JSON output.
