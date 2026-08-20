# API contract

## Accepted Authority selector forms

- `easynet:///r/<realm>/ability/authority.<name>@<version>#<digest>!<action>`
- `easynet:///r/<realm>/ability/authority.<name>`
- current Authority registry names such as `authority.binding.grant` or
  `meta.list_abilities` when used by daemon-internal catalog registration

## Rejected Authority selector forms

- `hub.<name>`

## Error contract

Rejection is `invalid_argument` at descriptor-wire construction before route
selection or descriptor lookup.
