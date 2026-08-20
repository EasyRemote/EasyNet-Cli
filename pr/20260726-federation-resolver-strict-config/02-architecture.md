# Architecture

`ResolverConfig` is the typed input to federation realm resolution:

```text
config file / tests
  -> ResolverConfig serde boundary
  -> resolve(realm, config)
  -> RealmResolution
  -> caller-owned federation route policy
```

The serde boundary must fail closed before resolution. Accepting unknown config
fields creates a hidden compatibility layer outside the resolver state machine.
