# Architecture

`RemoteInvocationSubject` is provenance state, not a string wrapper.

The state names must describe who owns the subject decision:

- `CallerDeclared`: public tuple input selected by the caller before dispatch.
- `DaemonTargetOwned`: daemon system/root issuer input selected from a
  target-owned runtime policy.

This keeps the seven-field tuple explicit while preserving the existing public
interfaces.
