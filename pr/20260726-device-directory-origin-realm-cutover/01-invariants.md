# Invariants

1. Federation directory rows own realm provenance under `origin_realm`.
2. CLI projection may adapt display state, but must not rename canonical
   provenance facts into retired product terminology.
3. JSON consumers must see one provenance field, not dual aliases.
4. Unknown or missing lifecycle facts still fail closed as before.
