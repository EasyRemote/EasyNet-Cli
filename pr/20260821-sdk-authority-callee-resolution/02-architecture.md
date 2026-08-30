# Architecture

## Layer boundary

- Axon/runtime SDK owns the generic runtime model, descriptor reference resolution, authority metadata shape, Invocation tuple construction, and conformance evidence.
- EasyNet-Cli daemon owns product runtime policy, local catalog publication, route resolution, plugin execution, and device-sponsored SystemAgent publication.
- EasyNet backend is a downstream SDK consumer. It must not mint Device-as-callee authority fixtures.

## Core model

```text
DescriptorResolution
  -> descriptor_ref
  -> resolved_callee_ura

AuthorizedRuntimeSession.prepare
  RuntimeTargetRef.URA        = selected execution target
  DescriptorResolution.owner  = callable callee
  InvocationDraft.callee_ura  = resolved_callee_ura
```

## Python parity fix

Go already normalizes catalogue descriptor provider requests through `newRuntimeCatalogueReadTarget`. Python now mirrors that behavior in `RuntimeClient.resolve_descriptor_ref` and `RuntimeAbilityClient`:

```text
input callee_ura: easynet:///r/<realm>/device/<id>
provider: ability_descriptor
ability: meta.list_abilities / meta.list_resources

wire callee_ura: easynet:///r/<realm>/agent/device.<id>.runtime-introspection
wire subject_ura: canonical runtime governance read subject
draft callee_ura: same SystemAgent
```

## Why public `callee_ura` is not renamed here

`RuntimeDescriptorRefRequest.callee_ura` is already public conformance-tracked SDK surface. Renaming it to `target_ura` would be a separate SPEC-level compatibility migration. This slice fixes the internal architectural behavior while preserving public behavior.
