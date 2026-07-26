# Architecture

The expected path is:

```text
SDK RuntimeClient
  -> canonical invocation transport
  -> daemon LocalRuntime/remote route
  -> descriptor provider
  -> admission authority
  -> signed receipt chain
```

No frontend, product read-model, or hub lifecycle code may create a parallel
descriptor/admission authority.
