1. Backend product code maps an authenticated account User URA to
   PrincipalLifecycle through the Go SDK Principal client.
2. The live E2E starts or attaches to exactly one daemon runtime through the
   public Go SDK native runtime provider.
3. PrincipalLifecycle mutations must be observed through daemon
   `principal.lifecycle.*` abilities, not through Backend-local fakes.
4. The public key crosses the boundary; no private key, seed or key-service
   custody field may enter Backend.
5. The test is tagged because C ABI daemon lifecycle support is an optional Go
   SDK provider build.
