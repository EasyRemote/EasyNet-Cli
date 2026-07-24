# Invariants

1. Plugin sidecar execution is a canonical runtime provider capability, not an EasyNet product capability.
2. Public helper APIs keep their semantic names and behavior; only product-specific package ownership changes.
3. Generated templates must consume SDK helper APIs, not hand-written protocol JSON.
4. No compatibility package remains under `provider/easynet/pluginexec`.
