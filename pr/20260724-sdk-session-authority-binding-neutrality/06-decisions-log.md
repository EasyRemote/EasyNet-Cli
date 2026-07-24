# Decisions

## DEC-1: Facade rename, generated schema unchanged

Generated Axon schema remains unchanged in this repository. The SDK facade owns
the product-neutral projection and maps generated provider fields into generic
runtime fields.

## DEC-2: No alias fallback

The SDK validators will not accept both old and new facade field names. That
would preserve the legacy architecture. Provider projections must emit the
canonical generic facade shape.

## DEC-3: Split provider schema from facade proof JSON

DirectRuntime providers may still read generated Axon `SessionAuthority`
fields named `backend_ura` and `user_ura` at the protobuf boundary. SDK
receipt facade JSON must expose `issuer_ura` and `subject_ura` only. Python
DirectRuntime keeps Axon canonical receipt verification on generated schema
field names while returning the generic facade projection to SDK consumers.
