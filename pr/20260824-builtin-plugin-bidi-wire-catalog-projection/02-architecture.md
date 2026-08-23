# Architecture

`BuiltinPluginAbilitySpec::to_registry_manifest` is the single adapter used by
builtin Remote Desktop and Browser contributions. It already projects
admission, frontend contract, and subject scope. This change adds the missing
typed `bidi_wire_kind` projection there, keeping plugin-specific registration
modules free of duplicate manifest mutation.

The daemon descriptor constructor continues to serialize the manifest field
into descriptor metadata. EasyNet backend and frontend already preserve and
validate that metadata, so no downstream fallback is needed.
