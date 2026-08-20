Runtime identity invariants

1. The SDK never discovers a product daemon endpoint implicitly.
2. The SDK never reads vault files, seed bytes, or private key material.
3. Runtime signing identity resolution requires both owner URA and daemon
   key-service socket path.
4. `LoadRuntimeSigningIdentity` resolves an existing daemon-owned public key
   projection before exposing a signer.
5. `EnsureRuntimeSigningIdentity` delegates key generation to the daemon
   key-service and returns only the public projection.
6. `RuntimeSigningIdentity.Sign` verifies returned signatures against the bound
   public projection before releasing the signature.
7. Public inventory must not list unusable compatibility APIs as canonical
   runtime identity capability in either Go or Python.
