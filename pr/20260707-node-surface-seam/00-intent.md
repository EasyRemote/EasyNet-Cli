# Node Surface Seam Intent

Add a Node/TypeScript Surface profile seam that follows
`docs/spec/daemon-sdk-requirements-v1.md` without introducing product rendering
or backend route policy into the SDK.

## Scope

- Expose Node Surface carriers for list/create/delete/manifest/health page
  operations.
- Delegate Invocation carrier construction and page operations to an injected
  Surface transport.
- Project daemon-authored page records, page pages, manifests, public refs,
  mutation results, and health/status facts into typed DTOs.
- Declare Node for `surface/page_carriers` only with direct Node test evidence.

## Out Of Scope

- No HTML rendering, frontend routing, CDN/cache policy, browser sessions, or
  backend public page serving.
- No direct filesystem page transport in Node.
- No DescriptorRef concatenation or local page ability grammar.
