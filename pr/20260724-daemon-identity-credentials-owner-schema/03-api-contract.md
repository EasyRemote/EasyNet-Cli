# API Contract

## Inputs

`credentials.json` is parsed exclusively as `persistence::config::Credentials`.

## Errors

- Missing file: returns absent daemon identity.
- Unknown, retired, malformed, or incomplete credentials: returns an error from the canonical credentials loader.
- Missing signer for the derived device URA: returns signer custody error.

## Compatibility

No boot-local compatibility layer is retained. Operators with old credentials must re-pair to create a canonical file.
