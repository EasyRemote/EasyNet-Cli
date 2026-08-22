# RemoteApp input scope visibility gate

## Intent

Add a product-flow gate for the frontend RemoteApp session details panel to
prove that input scope and pointer/keyboard enablement are visible to the user.

This closes an observability seam: daemon input readiness already carries
`input_scope`, `pointer_enabled`, and `keyboard_enabled`, but product evidence
must require the UI to show those facts instead of only showing a requested
interactive label.

## Non-goals

- Do not mark RemoteApp product completion.
- Do not treat visible scope as proof of successful OS input injection.
- Do not bypass daemon-owned authority, session lifecycle, or receipts.
