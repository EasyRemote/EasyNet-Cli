# Invariants

1. Runtime identity/addressing concepts use URA terminology only.
2. gRPC transport locator types may be used only behind the
   `GrpcEndpointLocator` alias.
3. Tests must follow the same transport naming boundary as production source.
4. HTTP request `.uri(...)` builder APIs remain transport-library calls and do
   not define runtime identity vocabulary.
