# Intent

Converge the Python `RuntimeClient.resolve_descriptor_ref` facade with the Go
runtime facade for omitted or whitespace-only descriptor call modes.

The runtime provider owns descriptor selection. A missing optional call mode
therefore has one canonical state: `rpc`. The provider must never infer an
empty mode independently for one language binding.

Removal condition: no Python runtime descriptor request may carry an empty or
whitespace-only `call_mode` value.
