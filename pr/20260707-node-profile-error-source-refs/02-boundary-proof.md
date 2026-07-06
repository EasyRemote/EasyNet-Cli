# Node Profile Error Source Refs Boundary Proof

## Ownership

The shared daemon error schema owns top-level error fields. Language facades own
ergonomic accessors and stable package source refs for errors that originate in
their profile validation boundary.

## Rejected Designs

- Adding `profile` or `source_ref` as new top-level fields: rejected because it
  would fork the shared typed error schema.
- Encoding profile source refs only in human-readable messages: rejected because
  callers need machine-readable details.
- Using product-specific source refs: rejected because SDK profiles are generic
  runtime/profile concepts, not EasyRemote or backend concepts.

## Call Path

```text
Profile client validation
  -> invalidProfile(profile, message, details)
  -> SDKError(details.profile + details.source_ref)
  -> SDKError.profile()/sourceRef()/errorClass()
```
