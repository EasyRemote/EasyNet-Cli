# API Contract

## Descriptor resolution request

Current public wire:

```json
{
  "callee_ura": "...",
  "ability": "...",
  "call_mode": "rpc|stream|bidi",
  "caller_ura": "...",
  "subject_ura": "...",
  "provider": "ability_descriptor|receipt_history"
}
```

Compatibility rule:

- The field name remains `callee_ura`.
- For generic public invocation, it must already be the callable owner.
- For catalogue descriptor provider reads, SDK facades may accept a Device execution target and must project it to the runtime-introspection SystemAgent before transport/draft construction.

## Authorized runtime session

`DescriptorResolution` now includes:

```text
resolved_callee_ura
```

If the provider does not fill it, SDKs derive it from the descriptor ref owner. The prepared Invocation draft uses this value as `callee_ura`.

## Error contract

- Missing descriptor-resolution provider caller/subject fields fail with `INVALID_ARGUMENT`.
- Device concrete authority targets fail with `INVALID_ARGUMENT`.
- Descriptor refs whose owner cannot project to Agent/Service/Authority fail before authority binding.
