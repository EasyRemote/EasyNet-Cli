# Architecture

## Boundary

`RuntimeDescriptorProviderKind` is the native provider boundary for descriptor resolution. It already owns provider parsing, source naming, ability admission, and receipt-history subject validation. This change completes that abstraction by moving ability descriptor subject validation into the same provider-kind boundary.

## Layering

- Core URA parsing remains Axon-owned through `crate::core::ura::parse_ura`.
- FFI performs request-shape and provider-policy validation.
- Daemon routing/admission receives only provider-admitted descriptor resolution requests.
- Go/Python SDK providers remain consumers of the same canonical runtime model.

## Removed legacy behavior

The previous `AbilityDescriptor => Ok(())` branch allowed target-owned catalogue subjects and deferred semantic failure to later daemon layers. That behavior is removed rather than preserved as a fallback.
