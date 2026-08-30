# Invariants

- Cleanup applies only to a direct `tmp/easynet-ability-deploy/*.tar.gz`
  ResourceRef materialized on the local Device.
- Directory bundles and arbitrary caller-owned archives are never deleted.
- Manifest bytes are fully parsed before cleanup.
- Cleanup succeeds before registrar mutation; failure cannot produce a durable
  install while leaving the staging lifecycle ambiguous.
- ResourceRef validation and root-escape protection remain owned by the
  filesystem resource provider.
