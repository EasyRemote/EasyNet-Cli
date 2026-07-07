# Boundary Proof

## SDK-Owned

- Surface request DTO validation and JSON serialization.
- Surface page/manifest/ref/mutation/health projection DTOs.
- Client lifecycle over an injected transport.
- Public TypeScript declarations for the same seam.

## Provider-Owned

- Ability DescriptorRef selection for `pages.list`, `pages.publish`,
  `pages.get`, `pages.unpublish`, and `pages.health`.
- Daemon page read model and page mutation semantics.
- Surface health checks and descriptor provenance.

## Product-Owned

- HTML rendering.
- Public HTTP routes.
- Browser auth/session handling.
- CDN/cache/storage policy.
- Product content management UX.

## Rejected Designs

- Direct filesystem page publication in Node: rejected because publication
  policy belongs to daemon/provider layers.
- DescriptorRef construction in Node: rejected because identity/daemon helpers
  own descriptor projection.
- Surface-specific runtime lifecycle: rejected because Surface is a profile
  client over the shared runtime model.
