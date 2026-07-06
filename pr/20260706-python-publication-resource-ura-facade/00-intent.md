# Intent

Remove the legacy Python Publication resource URA compatibility path.

Publication host resource references must be built through the canonical
Directory + Identity facade shape: `resource_ura(owner_ura, path)`. The
Publication profile must not fall back to older `resource_ura(realm, owner_id,
path)` call shapes or assemble owner ids such as `device.<node_id>`.
