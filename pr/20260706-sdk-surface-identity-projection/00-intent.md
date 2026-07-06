# Intent

Remove static URA construction from the Go Surface runtime projection path.

The Surface profile may normalize daemon `pages.*` facts into SDK DTOs, but it
must not become another owner of resource or agent URA grammar. When daemon
output omits `surface_ref`, the runtime facade may ask the Identity profile to
build a canonical resource URA from the already-known owner and page id instead
of formatting `easynet:///r/...` strings locally. If `owner_ura` is absent from
both daemon output and the request carrier, the projection must fail closed
rather than deriving an owner from `user` or `realm` strings.
