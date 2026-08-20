# Intent

## Goal

Move cross-language plugin execution sidecar helpers out of SDK `provider/easynet` namespaces into product-neutral `provider/runtime` namespaces.

## Non-goals

- Do not change plugin sidecar wire protocol behavior.
- Do not keep EasyNet import aliases.
- Do not migrate the separate Go lifecycle/identity provider facade in this iteration.

## Acceptance criteria

- Python, Go, Rust, Java, and Node pluginexec helpers use runtime provider package paths.
- Product plugin templates import the runtime helper packages.
- SPEC v2 gates assert runtime-provider ownership and reject `provider/easynet/pluginexec`.
- Existing pluginexec tests and template tests pass.
