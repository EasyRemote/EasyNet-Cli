# API Contract

## Public behavior

- Existing CLI/product commands should keep their public command shape.
- Error reporting should become more precise when legacy compatibility masking is removed.

## Runtime facts

- Caller URA, callee URA, subject URA, ability name/URA, action, and nonce must be explicit before daemon admission.
- Descriptor and route failures remain canonical runtime failures.

## Tenant rules

- URA terminology is mandatory.
- Product-specific directory, receipt, or lifecycle concepts cannot enter SDK public abstractions.
