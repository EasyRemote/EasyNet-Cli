# Intent

Close the admission local-self/off-box transport fork.

The same daemon invocation service can be served over local-only IPC and
off-box TCP/TLS. Local self admission is valid only on the local IPC boundary,
so this slice replaces the implicit boolean loopback policy with an explicit
transport-boundary state.
