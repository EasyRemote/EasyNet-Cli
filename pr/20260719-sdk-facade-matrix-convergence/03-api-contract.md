# API Contract

## Provider Helper Contract

Provider helpers must:

- read exactly one request frame from stdin;
- dispatch by ability name to a typed handler registry;
- return exactly one response frame on stdout;
- map handler errors into structured response errors;
- avoid product-specific naming outside the EasyNet provider namespace.

## Template Contract

Generated plugin templates must:

- import the provider helper for their language;
- register handlers through the helper's public API;
- avoid naked `json.loads`, `json.NewDecoder`, `JSON.parse(stdin)`, or
  equivalent ad hoc frame decoding.

## Public Compatibility

Existing public CLI behavior remains compatible for supported languages.
Unsupported or seam languages are rejected before template generation with an
explicit capability error.
