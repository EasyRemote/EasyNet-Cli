# Architecture

`descriptor_ref::ability_ura_for_wire` is a boundary helper shared by daemon
transport, LocalRuntime dispatch, FFI, and mission gateways. If it accepts
Authority-owned `hub.*` bare names such as `hub.openai.list_models`, then every
caller inherits a second product-era selector model.

The converged rule:

- complete descriptor ref: accepted after callee/owner match;
- canonical Ability URA: accepted after callee/owner match;
- Device bare dispatch name: projected for local daemon registry execution;
- Agent bare dispatch name: projected for local daemon registry execution;
- Authority bare dispatch name: accepted only as the literal current registry
  key; the retired `hub.` alias is rejected.

Authority ability calls must not rely on Hub-prefixed alias projection.
