# Invariants

1. A resident EasyRemote host using `easyremote._host.protocol` must be invoked
   through `host_stream` `protocol = binary_v1`.
2. EOF/reset before a terminal host frame remains a failure; the daemon must
   not reinterpret protocol mismatch as a clean stream close.
3. Cross-device native ability smoke must prove both handle call and stream
   paths over the same explicit protocol declaration.
4. EasyRemote-owned abilities must be removed by the EasyRemote provider
   lifecycle (`ComputeNode.stop()`), not by externally uninstalling a single
   ability while the provider's binding-lease renewer is still asserting that
   ability as desired state.
5. Manual `ability.uninstall` coverage remains required for CLI-deployed
   native abilities that are not owned by a live EasyRemote renewal lifecycle.
