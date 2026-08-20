# Execution Checklist

- [x] Move target-owned subject derivation into `RemoteSystemInvocationIssuer`.
- [x] Add a receipt-history selector guard at the target-owned remote system issuer boundary.
- [x] Migrate CLI and federation probe callers to the issuer-owned target-owned plan constructor.
- [x] Add tests proving the issuer rejects history selectors before constructing `DaemonTargetOwned`.
- [x] Update SPEC v2 and daemon invocation gates to reject direct history routing through target-owned remote system dispatch.
- [x] Run targeted tests.
- [x] Run canonical runtime convergence and architecture gates.
- [ ] Commit with the required author.
