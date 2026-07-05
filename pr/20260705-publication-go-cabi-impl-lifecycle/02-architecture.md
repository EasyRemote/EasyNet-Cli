# Architecture

Existing source of truth:

```text
src/daemon/publication_contract.rs
  -> src/ffi/publication/mod.rs
  -> include/easynet_cli.h
  -> sdk/go/cabi_publication.go
  -> PublicationClient
```

The missing edge is only the last projection edge for Go C ABI. The Rust
contract and C ABI already expose:

- `easynet_publication_build_enable_ability_impl_invocation`
- `easynet_publication_project_enable_ability_impl_result`
- `easynet_publication_build_disable_ability_impl_invocation`
- `easynet_publication_project_disable_ability_impl_result`

Go should bind those symbols and reuse the existing `invokeAndProject` helper.
