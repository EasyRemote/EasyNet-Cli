# Host Context Causal Verification

## Gates

- Python SDK Host Binding tests cover parent receipt preservation and malformed
  parent receipt rejection.
- EasyRemote Context tests cover disabled-without-anchor behavior and child
  dispatch with an SDK-projected causal anchor.
- EasyRemote host tests cover daemon envelope parent receipt propagation into
  `Context.call`.
- EasyRemote cutover audit remains clean.
- Full Python SDK and EasyRemote test suites pass.
- The daemon SDK requirements spec remains unchanged.

## Remaining Work

- Full Axon-backed receipt cryptographic verification still depends on full
  receipt body fetch and RFC-007 receipt URA resolution.
- EasyRemote still retains local Invocation DTO ergonomics until Runtime Core
  extraction fully absorbs prepare/sign/submit and typed Invocation result DTOs.
