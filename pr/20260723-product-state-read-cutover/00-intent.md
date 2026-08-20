# Intent

Move remaining product-state read projections off the generic local ability
action helper.

`skill.list` and `<user>.api_key.list` are read projections over daemon-owned
product state. They should not inherit the same local action invocation
authority path used by mutations such as `skill.install`, `skill.remove`,
`api_key.create`, and `api_key.revoke`.

This slice narrows the legacy local invocation surface without changing public
CLI flags or output.
