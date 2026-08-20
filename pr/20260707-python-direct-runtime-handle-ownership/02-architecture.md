# Architecture

`DirectDaemonRuntimeConnector` is the owner boundary for shared direct runtime connections. It resolves daemon endpoints, opens direct gRPC transports, and optionally owns one shared handle transport used by connector-created transports.

`DirectDaemonRuntimeTransport` remains a direct gRPC transport for unary, stream, and bidi invocation operations. It may delegate prepared/signed handle operations to a configured handle transport. Connector-created transports are non-owning delegates; standalone transports can be configured to own their delegate.

This preserves a single runtime capability model across Go and Python:

- direct daemon gRPC: provider-backed for invoke, stream, and bidi
- handle transport delegation: provider-backed when supplied
- missing handle transport: unsupported for prepare, submit, and handle lifecycle

