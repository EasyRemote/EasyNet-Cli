# Architecture

## Root abstraction problem

The SDK currently exposes plugin execution helpers under `provider/easynet`, which makes EasyNet appear to contribute to the canonical runtime abstraction. That violates the SDK model: products consume the runtime SDK; they do not define SDK provider namespaces.

## Target model

- `provider/runtime/pluginexec` owns cross-language sidecar helpers.
- Product templates reference those helpers as SDK runtime provider packages.
- Gates prevent reintroducing product-named pluginexec packages in SDK helper paths.

## Boundary

This iteration migrates plugin execution helpers only. Go lifecycle/identity provider facades remain a separately scoped seam because their public lifecycle abstraction and tests need a separate cutover plan.
