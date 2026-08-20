# API Contract

- Wire protocol framing is unchanged.
- Handler/invocation/runtime class and function names are unchanged.
- Generated plugin templates continue to build against SDK helper packages.
- Import/package paths become product-neutral:
  - Python: `easynet_sdk.providers.runtime.plugin_exec`
  - Go: `easynet.run/cli/sdk/go/provider/runtime/pluginexec`
  - Rust: `sdk/rust/provider/runtime/pluginexec`
  - Java: `run.runtime.sdk.provider.runtime.pluginexec`
  - Node: `@runtime/sdk/provider/runtime/pluginexec.js`
